use std::path::PathBuf;
use thiserror::Error;

/// Unified error type across domain, application, and infrastructure layers.
#[derive(Debug, Error)]
pub enum MidxError {
    #[error(
        "midx repository not initialized (no .midx.toml found in '{0}' or parent directories); run `midx init` first"
    )]
    NotInitialized(PathBuf),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("file scanning failed: {0}")]
    Scanner(String),

    #[error("embedding generation failed: {0}")]
    Embedder(String),

    #[error("repository error: {0}")]
    Repository(String),

    #[error("corrupted embedding blob: byte length {0} is not divisible by 4")]
    InvalidEmbeddingBytes(usize),

    #[error("search query cannot be empty or whitespace-only")]
    EmptyQuery,
}
