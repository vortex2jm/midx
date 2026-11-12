use std::path::Path;

use crate::application::ports::{FileScanner, NoteRepository, TextEmbedder};
use crate::application::sync::SyncUseCase;
use crate::domain::errors::MidxError;
use crate::domain::models::SearchHit;
use crate::domain::similarity::top_k;

/// Semantic search use case with optional incremental pre-sync and Top-K ranking.
pub struct SearchUseCase<'a, S, E, R>
where
    S: FileScanner + ?Sized,
    E: TextEmbedder + ?Sized,
    R: NoteRepository + ?Sized,
{
    scanner: &'a S,
    embedder: &'a E,
    repo: &'a R,
    max_chunk_words: usize,
    batch_size: usize,
}

impl<'a, S, E, R> SearchUseCase<'a, S, E, R>
where
    S: FileScanner + ?Sized,
    E: TextEmbedder + ?Sized,
    R: NoteRepository + ?Sized,
{
    /// Constructs a `SearchUseCase` from injected ports.
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
            max_chunk_words,
            batch_size,
        }
    }

    /// Runs an optional incremental sync, embeds `query`, and returns the top `limit` hits.
    pub fn execute(
        &self,
        root: &Path,
        query: &str,
        limit: usize,
        no_sync: bool,
    ) -> Result<Vec<SearchHit>, MidxError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Err(MidxError::EmptyQuery);
        }

        if !no_sync {
            let sync = SyncUseCase::new(
                self.scanner,
                self.embedder,
                self.repo,
                self.max_chunk_words,
                self.batch_size,
            );
            let _ = sync.execute(root, false)?;
        }

        let chunks = self.repo.get_all_chunks()?;
        if chunks.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let query_embedding = self.embedder.embed_query(trimmed)?;
        Ok(top_k(&query_embedding, &chunks, limit))
    }
}
