use std::collections::HashMap;
use std::path::Path;

use crate::domain::errors::MidxError;
use crate::domain::models::{
    EmbeddedChunk, Embedding, IndexStats, IndexedFile, ScannedFile, StoredChunk,
};

/// Port for discovering Markdown files and reading their contents from a source tree.
pub trait FileScanner {
    /// Recursively scans `root` for `.md` files, ignoring hidden entries and `.git/`.
    fn scan(&self, root: &Path) -> Result<Vec<ScannedFile>, MidxError>;

    /// Reads the UTF-8 contents of a file identified by its repository-relative path.
    fn read_content(&self, root: &Path, relative_path: &str) -> Result<String, MidxError>;
}

/// Port for generating dense semantic vector embeddings.
pub trait TextEmbedder {
    /// Generates embeddings for a batch of document passages.
    fn embed_passages(&self, passages: &[String]) -> Result<Vec<Embedding>, MidxError>;

    /// Generates an embedding for a search query.
    fn embed_query(&self, query: &str) -> Result<Embedding, MidxError>;
}

/// Port for persisting indexed files, semantic chunks, and vector embeddings.
pub trait NoteRepository {
    /// Returns a map of `relative_path -> IndexedFile` for all currently indexed files.
    fn get_indexed_files(&self) -> Result<HashMap<String, IndexedFile>, MidxError>;

    /// Atomically replaces all chunks and file metadata for a single file (Unit of Work).
    fn save_file_chunks(
        &self,
        file: &ScannedFile,
        chunks: &[EmbeddedChunk],
    ) -> Result<(), MidxError>;

    /// Deletes files (and cascades to their chunks) for the given relative paths.
    fn delete_files(&self, paths: &[String]) -> Result<usize, MidxError>;

    /// Retrieves all stored chunks with their embeddings for similarity search.
    fn get_all_chunks(&self) -> Result<Vec<StoredChunk>, MidxError>;

    /// Returns aggregate index statistics.
    fn get_stats(&self) -> Result<IndexStats, MidxError>;
}

