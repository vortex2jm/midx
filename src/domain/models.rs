use std::fmt;
use std::time::Duration;

use crate::domain::errors::MidxError;

/// Newtype wrapper around a hex-encoded SHA-256 content digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Hash(String);

impl Sha256Hash {
    /// Creates a new `Sha256Hash` from a hex string.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the underlying hash string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Sha256Hash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Newtype wrapper around a dense `f32` embedding vector with safe little-endian serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedding(Vec<f32>);

impl Embedding {
    /// Wraps a vector of `f32` values as an `Embedding`.
    pub fn new(values: Vec<f32>) -> Self {
        Self(values)
    }

    /// Returns a slice view over the embedding coordinates.
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// Returns `true` if the embedding has zero dimensions.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Serializes the embedding to a little-endian byte buffer.
    pub fn to_le_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.0.len() * std::mem::size_of::<f32>());
        for &value in &self.0 {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// Deserializes an `Embedding` from a little-endian byte slice.
    pub fn from_le_bytes(bytes: &[u8]) -> Result<Self, MidxError> {
        if !bytes.len().is_multiple_of(4) {
            return Err(MidxError::InvalidEmbeddingBytes(bytes.len()));
        }

        let (chunks, remainder) = bytes.as_chunks::<4>();
        if !remainder.is_empty() {
            return Err(MidxError::InvalidEmbeddingBytes(bytes.len()));
        }

        let values = chunks
            .iter()
            .map(|&chunk| f32::from_le_bytes(chunk))
            .collect();

        Ok(Self(values))
    }
}

/// Metadata and content digest of a Markdown file discovered on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    pub path: String,
    pub hash: Sha256Hash,
    pub mtime: i64,
}

/// Persisted state of an indexed Markdown file in the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFile {
    pub path: String,
    pub hash: Sha256Hash,
    pub mtime: i64,
}

/// A semantic chunk extracted from a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunk {
    pub heading_context: String,
    pub content: String,
    pub line_start: usize,
}

/// A chunk paired with its computed dense vector embedding.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedChunk {
    pub chunk: TextChunk,
    pub embedding: Embedding,
}

/// A chunk retrieved from persistent storage for similarity ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredChunk {
    pub id: i64,
    pub file_path: String,
    pub heading_context: String,
    pub content: String,
    pub line_start: usize,
    pub embedding: Embedding,
}

/// A ranked semantic search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub score: f32,
    pub file_path: String,
    pub line_start: usize,
    pub heading_context: String,
    pub content: String,
}

/// Summary statistics produced by an incremental sync run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSummary {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub chunks_indexed: usize,
    pub elapsed: Duration,
}

/// Aggregate index statistics for the `status` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStats {
    pub total_files: usize,
    pub total_chunks: usize,
    pub db_size_bytes: u64,
}
