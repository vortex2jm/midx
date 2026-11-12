use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::application::ports::NoteRepository;
use crate::domain::errors::MidxError;
use crate::domain::models::{
    EmbeddedChunk, Embedding, IndexStats, IndexedFile, ScannedFile, Sha256Hash, StoredChunk,
};

/// SQLite-backed `NoteRepository` using WAL mode, foreign key cascades,
/// and per-file atomic transactions.
pub struct SqliteStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl SqliteStore {
    /// Opens (or creates) the SQLite database at `db_path`, configures PRAGMAs,
    /// and ensures the schema exists.
    pub fn open(db_path: &Path) -> Result<Self, MidxError> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path).map_err(map_sqlite_err)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )
        .map_err(map_sqlite_err)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                hash TEXT NOT NULL,
                mtime INTEGER NOT NULL,
                indexed_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
                heading_context TEXT NOT NULL,
                content TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                embedding BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_chunks_file_path ON chunks(file_path);",
        )
        .map_err(map_sqlite_err)?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: db_path.to_path_buf(),
        })
    }

    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, MidxError> {
        self.conn
            .lock()
            .map_err(|_| MidxError::Repository("sqlite connection mutex poisoned".to_string()))
    }
}

impl NoteRepository for SqliteStore {
    fn get_indexed_files(&self) -> Result<HashMap<String, IndexedFile>, MidxError> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT path, hash, mtime FROM files")
            .map_err(map_sqlite_err)?;

        let rows = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                let hash: String = row.get(1)?;
                let mtime: i64 = row.get(2)?;
                Ok(IndexedFile {
                    path,
                    hash: Sha256Hash::new(hash),
                    mtime,
                })
            })
            .map_err(map_sqlite_err)?;

        let mut map = HashMap::new();
        for row_res in rows {
            let file = row_res.map_err(map_sqlite_err)?;
            map.insert(file.path.clone(), file);
        }
        Ok(map)
    }

    fn save_file_chunks(
        &self,
        file: &ScannedFile,
        chunks: &[EmbeddedChunk],
    ) -> Result<(), MidxError> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(map_sqlite_err)?;

        let indexed_at = current_unix_seconds();

        tx.execute("DELETE FROM chunks WHERE file_path = ?1", params![file.path])
            .map_err(map_sqlite_err)?;

        tx.execute(
            "INSERT INTO files (path, hash, mtime, indexed_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
                hash = excluded.hash,
                mtime = excluded.mtime,
                indexed_at = excluded.indexed_at",
            params![file.path, file.hash.as_str(), file.mtime, indexed_at],
        )
        .map_err(map_sqlite_err)?;

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO chunks (file_path, heading_context, content, line_start, embedding)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(map_sqlite_err)?;

            for item in chunks {
                let line_start = i64::try_from(item.chunk.line_start).map_err(|_| {
                    MidxError::Repository("line_start exceeds SQLite integer range".to_string())
                })?;
                let blob = item.embedding.to_le_bytes();
                stmt.execute(params![
                    file.path,
                    item.chunk.heading_context,
                    item.chunk.content,
                    line_start,
                    blob
                ])
                .map_err(map_sqlite_err)?;
            }
        }

        tx.commit().map_err(map_sqlite_err)?;
        Ok(())
    }

    fn delete_files(&self, paths: &[String]) -> Result<usize, MidxError> {
        if paths.is_empty() {
            return Ok(0);
        }

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(map_sqlite_err)?;
        let mut deleted = 0usize;

        {
            let mut stmt = tx
                .prepare("DELETE FROM files WHERE path = ?1")
                .map_err(map_sqlite_err)?;

            for path in paths {
                deleted += stmt.execute(params![path]).map_err(map_sqlite_err)?;
            }
        }

        tx.commit().map_err(map_sqlite_err)?;
        Ok(deleted)
    }

    fn get_all_chunks(&self) -> Result<Vec<StoredChunk>, MidxError> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, file_path, heading_context, content, line_start, embedding
                 FROM chunks",
            )
            .map_err(map_sqlite_err)?;

        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let file_path: String = row.get(1)?;
                let heading_context: String = row.get(2)?;
                let content: String = row.get(3)?;
                let line_start: i64 = row.get(4)?;
                let blob: Vec<u8> = row.get(5)?;
                Ok((id, file_path, heading_context, content, line_start, blob))
            })
            .map_err(map_sqlite_err)?;

        let mut result = Vec::new();
        for row_res in rows {
            let (id, file_path, heading_context, content, line_start, blob) =
                row_res.map_err(map_sqlite_err)?;
            let embedding = Embedding::from_le_bytes(&blob)?;
            let line_start_usize = usize::try_from(line_start).unwrap_or(1);

            result.push(StoredChunk {
                id,
                file_path,
                heading_context,
                content,
                line_start: line_start_usize,
                embedding,
            });
        }

        Ok(result)
    }

    fn get_stats(&self) -> Result<IndexStats, MidxError> {
        let conn = self.lock_conn()?;
        let total_files: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .map_err(map_sqlite_err)?;
        let total_chunks: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .map_err(map_sqlite_err)?;

        let db_size_bytes = fs::metadata(&self.db_path).map(|m| m.len()).unwrap_or(0);

        Ok(IndexStats {
            total_files: usize::try_from(total_files).unwrap_or(0),
            total_chunks: usize::try_from(total_chunks).unwrap_or(0),
            db_size_bytes,
        })
    }
}

fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn map_sqlite_err(err: rusqlite::Error) -> MidxError {
    MidxError::Repository(err.to_string())
}

