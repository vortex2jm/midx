use std::sync::Mutex;

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::application::ports::TextEmbedder;
use crate::domain::errors::MidxError;
use crate::domain::models::Embedding;

/// Local ONNX embedding adapter powered by `fastembed`.
pub struct OnnxEmbedder {
    model: Mutex<TextEmbedding>,
}

impl OnnxEmbedder {
    /// Initializes an ONNX `TextEmbedding` model by name (e.g. `"MultilingualE5Small"`).
    pub fn new(model_name: &str, show_download_progress: bool) -> Result<Self, MidxError> {
        let embedding_model = parse_embedding_model(model_name)?;
        let options = TextInitOptions::new(embedding_model)
            .with_show_download_progress(show_download_progress);

        let model = TextEmbedding::try_new(options)
            .map_err(|err| MidxError::Embedder(err.to_string()))?;

        Ok(Self {
            model: Mutex::new(model),
        })
    }
}

impl TextEmbedder for OnnxEmbedder {
    fn embed_passages(&self, passages: &[String]) -> Result<Vec<Embedding>, MidxError> {
        if passages.is_empty() {
            return Ok(Vec::new());
        }

        let prefixed: Vec<String> = passages
            .iter()
            .map(|text| format!("passage: {text}"))
            .collect();

        let mut guard = self
            .model
            .lock()
            .map_err(|_| MidxError::Embedder("embedder mutex poisoned".to_string()))?;

        let vectors = guard
            .embed(prefixed, Some(passages.len()))
            .map_err(|err| MidxError::Embedder(err.to_string()))?;

        Ok(vectors.into_iter().map(Embedding::new).collect())
    }

    fn embed_query(&self, query: &str) -> Result<Embedding, MidxError> {
        let prefixed = format!("query: {query}");

        let mut guard = self
            .model
            .lock()
            .map_err(|_| MidxError::Embedder("embedder mutex poisoned".to_string()))?;

        let mut vectors = guard
            .embed(vec![prefixed], None)
            .map_err(|err| MidxError::Embedder(err.to_string()))?;

        vectors
            .pop()
            .map(Embedding::new)
            .ok_or_else(|| MidxError::Embedder("model returned no embedding for query".to_string()))
    }
}

fn parse_embedding_model(name: &str) -> Result<EmbeddingModel, MidxError> {
    match name.trim() {
        "MultilingualE5Small" | "intfloat/multilingual-e5-small" => {
            Ok(EmbeddingModel::MultilingualE5Small)
        }
        "MultilingualE5Base" | "intfloat/multilingual-e5-base" => {
            Ok(EmbeddingModel::MultilingualE5Base)
        }
        "MultilingualE5Large" | "intfloat/multilingual-e5-large" => {
            Ok(EmbeddingModel::MultilingualE5Large)
        }
        other => Err(MidxError::Config(format!(
            "unsupported embedding model '{other}'; expected 'MultilingualE5Small'"
        ))),
    }
}

