use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use crate::application::ports::{FileScanner, NoteRepository, TextEmbedder};
use crate::domain::chunker::MarkdownChunker;
use crate::domain::errors::MidxError;
use crate::domain::models::{EmbeddedChunk, ScannedFile, SyncSummary, TextChunk};

/// Default number of chunks grouped into a single `embed_passages` batch.
pub const DEFAULT_BATCH_SIZE: usize = 32;

struct PendingFileState {
    file: ScannedFile,
    is_update: bool,
    expected_chunks: usize,
    embedded: Vec<EmbeddedChunk>,
}

struct QueuedChunk {
    file_idx: usize,
    chunk: TextChunk,
    passage_text: String,
}

/// Incremental synchronization use case combining diff detection, batched embedding,
/// and per-file atomic persistence.
pub struct SyncUseCase<'a, S, E, R>
where
    S: FileScanner + ?Sized,
    E: TextEmbedder + ?Sized,
    R: NoteRepository + ?Sized,
{
    scanner: &'a S,
    embedder: &'a E,
    repo: &'a R,
    chunker: MarkdownChunker,
    batch_size: usize,
}

impl<'a, S, E, R> SyncUseCase<'a, S, E, R>
where
    S: FileScanner + ?Sized,
    E: TextEmbedder + ?Sized,
    R: NoteRepository + ?Sized,
{
    /// Constructs a new `SyncUseCase` with injected ports and pipeline parameters.
    pub fn new(
        scanner: &'a S,
        embedder: &'a E,
        repo: &'a R,
        max_chunk_words: usize,
        batch_size: usize,
    ) -> Self {
        Self {
            scanner,
            embedder,
            repo,
            chunker: MarkdownChunker::new(max_chunk_words),
            batch_size: batch_size.max(1),
        }
    }

    /// Scans `root`, computes incremental changes against the repository,
    /// batches chunk embeddings, and commits each file atomically.
    pub fn execute(&self, root: &Path, force: bool) -> Result<SyncSummary, MidxError> {
        let started_at = Instant::now();

        let scanned_files = self.scanner.scan(root)?;
        let indexed_files = self.repo.get_indexed_files()?;

        let scanned_paths: HashSet<&str> =
            scanned_files.iter().map(|f| f.path.as_str()).collect();

        let deleted_paths: Vec<String> = indexed_files
            .keys()
            .filter(|path| !scanned_paths.contains(path.as_str()))
            .cloned()
            .collect();

        let removed = if deleted_paths.is_empty() {
            0
        } else {
            self.repo.delete_files(&deleted_paths)?
        };

        let mut unchanged = 0usize;
        let mut changed_files: Vec<(ScannedFile, bool)> = Vec::new();

        for file in scanned_files {
            match indexed_files.get(&file.path) {
                None => changed_files.push((file, false)),
                Some(existing) => {
                    let is_same =
                        !force && existing.mtime == file.mtime && existing.hash == file.hash;
                    let content_unchanged = !force && existing.hash == file.hash;
                    if is_same || content_unchanged {
                        unchanged += 1;
                    } else {
                        changed_files.push((file, true));
                    }
                }
            }
        }

        let mut pending_states: Vec<PendingFileState> = Vec::with_capacity(changed_files.len());
        let mut batch_queue: Vec<QueuedChunk> = Vec::with_capacity(self.batch_size);

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut chunks_indexed = 0usize;

        for (file, is_update) in changed_files {
            let content = self.scanner.read_content(root, &file.path)?;
            let chunks = self.chunker.chunk(&content);

            if chunks.is_empty() {
                self.repo.save_file_chunks(&file, &[])?;
                if is_update {
                    updated += 1;
                } else {
                    added += 1;
                }
                continue;
            }

            let file_idx = pending_states.len();
            let expected_chunks = chunks.len();
            pending_states.push(PendingFileState {
                file,
                is_update,
                expected_chunks,
                embedded: Vec::with_capacity(expected_chunks),
            });

            for chunk in chunks {
                let passage_text = format_passage_text(&chunk);
                batch_queue.push(QueuedChunk {
                    file_idx,
                    chunk,
                    passage_text,
                });

                if batch_queue.len() >= self.batch_size {
                    let stats = self.flush_batch(
                        &mut batch_queue,
                        &mut pending_states,
                        &mut added,
                        &mut updated,
                    )?;
                    chunks_indexed += stats;
                }
            }
        }

        if !batch_queue.is_empty() {
            chunks_indexed += self.flush_batch(
                &mut batch_queue,
                &mut pending_states,
                &mut added,
                &mut updated,
            )?;
        }

        Ok(SyncSummary {
            added,
            updated,
            removed,
            unchanged,
            chunks_indexed,
            elapsed: started_at.elapsed(),
        })
    }

    fn flush_batch(
        &self,
        batch_queue: &mut Vec<QueuedChunk>,
        pending_states: &mut [PendingFileState],
        added: &mut usize,
        updated: &mut usize,
    ) -> Result<usize, MidxError> {
        if batch_queue.is_empty() {
            return Ok(0);
        }

        let passages: Vec<String> = batch_queue.iter().map(|q| q.passage_text.clone()).collect();
        let embeddings = self.embedder.embed_passages(&passages)?;

        if embeddings.len() != batch_queue.len() {
            return Err(MidxError::Embedder(format!(
                "embedder returned {} vectors for {} passages",
                embeddings.len(),
                batch_queue.len()
            )));
        }

        let count = embeddings.len();
        let mut completed_indices = Vec::new();

        for (queued, embedding) in batch_queue.drain(..).zip(embeddings) {
            let state = pending_states.get_mut(queued.file_idx).ok_or_else(|| {
                MidxError::Repository("internal pending file index out of bounds".to_string())
            })?;

            state.embedded.push(EmbeddedChunk {
                chunk: queued.chunk,
                embedding,
            });

            if state.embedded.len() == state.expected_chunks {
                completed_indices.push(queued.file_idx);
            }
        }

        for idx in completed_indices {
            let state = pending_states.get(idx).ok_or_else(|| {
                MidxError::Repository("internal pending file index out of bounds".to_string())
            })?;
            self.repo.save_file_chunks(&state.file, &state.embedded)?;
            if state.is_update {
                *updated += 1;
            } else {
                *added += 1;
            }
        }

        Ok(count)
    }
}

fn format_passage_text(chunk: &TextChunk) -> String {
    if chunk.heading_context.is_empty() {
        chunk.content.clone()
    } else {
        format!("{}\n\n{}", chunk.heading_context, chunk.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    use crate::domain::models::{Embedding, IndexStats, IndexedFile, Sha256Hash, StoredChunk};

    struct MockScanner {
        files: RefCell<HashMap<String, (i64, String)>>,
    }

    impl MockScanner {
        fn new() -> Self {
            Self {
                files: RefCell::new(HashMap::new()),
            }
        }

        fn insert(&self, path: &str, mtime: i64, content: &str) {
            self.files
                .borrow_mut()
                .insert(path.to_string(), (mtime, content.to_string()));
        }

        fn remove(&self, path: &str) {
            self.files.borrow_mut().remove(path);
        }

        fn fake_hash(content: &str) -> Sha256Hash {
            Sha256Hash::new(format!("hash:{}:{}", content.len(), content))
        }
    }

    impl FileScanner for MockScanner {
        fn scan(&self, _root: &Path) -> Result<Vec<ScannedFile>, MidxError> {
            let mut result = self
                .files
                .borrow()
                .iter()
                .map(|(path, (mtime, content))| ScannedFile {
                    path: path.clone(),
                    hash: Self::fake_hash(content),
                    mtime: *mtime,
                })
                .collect::<Vec<_>>();
            result.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(result)
        }

        fn read_content(&self, _root: &Path, relative_path: &str) -> Result<String, MidxError> {
            self.files
                .borrow()
                .get(relative_path)
                .map(|(_, content)| content.clone())
                .ok_or_else(|| MidxError::Scanner(format!("missing mock file: {relative_path}")))
        }
    }

    struct MockEmbedder {
        batch_sizes: RefCell<Vec<usize>>,
    }

    impl MockEmbedder {
        fn new() -> Self {
            Self {
                batch_sizes: RefCell::new(Vec::new()),
            }
        }
    }

    impl TextEmbedder for MockEmbedder {
        fn embed_passages(&self, passages: &[String]) -> Result<Vec<Embedding>, MidxError> {
            self.batch_sizes.borrow_mut().push(passages.len());
            Ok(passages
                .iter()
                .enumerate()
                .map(|(idx, text)| Embedding::new(vec![text.len() as f32, idx as f32, 1.0]))
                .collect())
        }

        fn embed_query(&self, query: &str) -> Result<Embedding, MidxError> {
            Ok(Embedding::new(vec![query.len() as f32, 0.0, 1.0]))
        }
    }

    struct InMemoryRepo {
        files: RefCell<HashMap<String, IndexedFile>>,
        chunks: RefCell<HashMap<String, Vec<StoredChunk>>>,
        next_id: RefCell<i64>,
    }

    impl InMemoryRepo {
        fn new() -> Self {
            Self {
                files: RefCell::new(HashMap::new()),
                chunks: RefCell::new(HashMap::new()),
                next_id: RefCell::new(1),
            }
        }
    }

    impl NoteRepository for InMemoryRepo {
        fn get_indexed_files(&self) -> Result<HashMap<String, IndexedFile>, MidxError> {
            Ok(self.files.borrow().clone())
        }

        fn save_file_chunks(
            &self,
            file: &ScannedFile,
            chunks: &[EmbeddedChunk],
        ) -> Result<(), MidxError> {
            self.files.borrow_mut().insert(
                file.path.clone(),
                IndexedFile {
                    path: file.path.clone(),
                    hash: file.hash.clone(),
                    mtime: file.mtime,
                },
            );

            let mut stored = Vec::with_capacity(chunks.len());
            for item in chunks {
                let mut id_guard = self.next_id.borrow_mut();
                let id = *id_guard;
                *id_guard += 1;
                stored.push(StoredChunk {
                    id,
                    file_path: file.path.clone(),
                    heading_context: item.chunk.heading_context.clone(),
                    content: item.chunk.content.clone(),
                    line_start: item.chunk.line_start,
                    embedding: item.embedding.clone(),
                });
            }
            self.chunks.borrow_mut().insert(file.path.clone(), stored);
            Ok(())
        }

        fn delete_files(&self, paths: &[String]) -> Result<usize, MidxError> {
            let mut removed = 0usize;
            for path in paths {
                if self.files.borrow_mut().remove(path).is_some() {
                    self.chunks.borrow_mut().remove(path);
                    removed += 1;
                }
            }
            Ok(removed)
        }

        fn get_all_chunks(&self) -> Result<Vec<StoredChunk>, MidxError> {
            Ok(self
                .chunks
                .borrow()
                .values()
                .flat_map(|list| list.iter().cloned())
                .collect())
        }

        fn get_stats(&self) -> Result<IndexStats, MidxError> {
            Ok(IndexStats {
                total_files: self.files.borrow().len(),
                total_chunks: self.chunks.borrow().values().map(|v| v.len()).sum(),
                db_size_bytes: 0,
            })
        }
    }

    #[test]
    fn sync_handles_new_updated_unchanged_and_removed_files_with_batching() {
        let scanner = MockScanner::new();
        let embedder = MockEmbedder::new();
        let repo = InMemoryRepo::new();

        // 3 files, each with 2 sections -> 6 chunks total; with batch_size=4 we expect [4, 2]
        scanner.insert("a.md", 100, "# A1\nFirst.\n\n# A2\nSecond.");
        scanner.insert("b.md", 100, "# B1\nFirst.\n\n# B2\nSecond.");
        scanner.insert("c.md", 100, "# C1\nFirst.\n\n# C2\nSecond.");

        let use_case = SyncUseCase::new(&scanner, &embedder, &repo, 480, 4);

        // 1. Initial sync: all 3 added
        let summary1 = use_case
            .execute(Path::new("/virtual"), false)
            .expect("initial sync succeeds");
        assert_eq!(summary1.added, 3);
        assert_eq!(summary1.updated, 0);
        assert_eq!(summary1.removed, 0);
        assert_eq!(summary1.unchanged, 0);
        assert_eq!(summary1.chunks_indexed, 6);
        assert_eq!(*embedder.batch_sizes.borrow(), vec![4, 2]);
        assert_eq!(repo.get_stats().expect("stats").total_chunks, 6);

        // 2. Re-sync without changes: all skipped
        embedder.batch_sizes.borrow_mut().clear();
        let summary2 = use_case
            .execute(Path::new("/virtual"), false)
            .expect("noop sync succeeds");
        assert_eq!(summary2.added, 0);
        assert_eq!(summary2.updated, 0);
        assert_eq!(summary2.removed, 0);
        assert_eq!(summary2.unchanged, 3);
        assert_eq!(summary2.chunks_indexed, 0);
        assert!(embedder.batch_sizes.borrow().is_empty());

        // 3. Edit b.md, remove c.md, add d.md
        scanner.insert("b.md", 200, "# B1\nUpdated content only one section.");
        scanner.remove("c.md");
        scanner.insert("d.md", 300, "# D1\nBrand new note.");

        let summary3 = use_case
            .execute(Path::new("/virtual"), false)
            .expect("incremental sync succeeds");
        assert_eq!(summary3.added, 1);
        assert_eq!(summary3.updated, 1);
        assert_eq!(summary3.removed, 1);
        assert_eq!(summary3.unchanged, 1);
        assert_eq!(summary3.chunks_indexed, 2);

        let all_chunks = repo.get_all_chunks().expect("chunks");
        assert_eq!(all_chunks.len(), 4); // 2 from a.md + 1 from b.md + 1 from d.md
        assert!(all_chunks.iter().all(|c| c.file_path != "c.md"));

        // 4. Force sync re-indexes all 3 remaining files
        let summary4 = use_case
            .execute(Path::new("/virtual"), true)
            .expect("force sync succeeds");
        assert_eq!(summary4.added, 0);
        assert_eq!(summary4.updated, 3);
        assert_eq!(summary4.unchanged, 0);
        assert_eq!(summary4.chunks_indexed, 4);
    }
}

