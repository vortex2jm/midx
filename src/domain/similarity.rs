use crate::domain::models::{Embedding, SearchHit, StoredChunk};

/// Computes the cosine similarity in `[-1.0, 1.0]` between two dense vectors.
/// Returns `0.0` if vectors differ in length, are empty, or have zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;

    for (&x, &y) in a.iter().zip(b.iter()) {
        let x64 = f64::from(x);
        let y64 = f64::from(y);
        dot += x64 * y64;
        norm_a += x64 * x64;
        norm_b += y64 * y64;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 || !denom.is_finite() {
        0.0
    } else {
        (dot / denom) as f32
    }
}

/// Scores and returns the top `k` most similar chunks for a query embedding.
pub fn top_k(query: &Embedding, chunks: &[StoredChunk], k: usize) -> Vec<SearchHit> {
    if k == 0 || chunks.is_empty() || query.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<SearchHit> = chunks
        .iter()
        .map(|chunk| SearchHit {
            score: cosine_similarity(query.as_slice(), chunk.embedding.as_slice()),
            file_path: chunk.file_path.clone(),
            line_start: chunk.line_start,
            heading_context: chunk.heading_context.clone(),
            content: chunk.content.clone(),
        })
        .collect();

    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    scored.truncate(k);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::errors::MidxError;

    #[test]
    fn cosine_similarity_handles_identical_orthogonal_and_zero_vectors() {
        let a = [1.0, 2.0, 3.0];
        let b = [1.0, 2.0, 3.0];
        let sim_same = cosine_similarity(&a, &b);
        assert!((sim_same - 1.0).abs() < 1e-6);

        let ortho_a = [1.0, 0.0];
        let ortho_b = [0.0, 1.0];
        assert!(cosine_similarity(&ortho_a, &ortho_b).abs() < 1e-6);

        let opp_a = [1.0, 0.0];
        let opp_b = [-1.0, 0.0];
        assert!((cosine_similarity(&opp_a, &opp_b) + 1.0).abs() < 1e-6);

        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn embedding_little_endian_roundtrip_and_validation() {
        let original = Embedding::new(vec![0.125, -3.5, 42.0, 0.0]);
        let bytes = original.to_le_bytes();
        assert_eq!(bytes.len(), 16);

        let decoded = Embedding::from_le_bytes(&bytes).expect("valid byte slice");
        assert_eq!(decoded, original);

        let err = Embedding::from_le_bytes(&[1, 2, 3]).expect_err("must reject invalid length");
        assert!(matches!(err, MidxError::InvalidEmbeddingBytes(3)));
    }

    #[test]
    fn top_k_orders_descending_and_truncates() {
        let query = Embedding::new(vec![1.0, 0.0]);
        let chunks = vec![
            StoredChunk {
                id: 1,
                file_path: "low.md".to_string(),
                heading_context: "# Low".to_string(),
                content: "low".to_string(),
                line_start: 1,
                embedding: Embedding::new(vec![0.2, 0.98]),
            },
            StoredChunk {
                id: 2,
                file_path: "high.md".to_string(),
                heading_context: "# High".to_string(),
                content: "high".to_string(),
                line_start: 10,
                embedding: Embedding::new(vec![0.99, 0.1]),
            },
            StoredChunk {
                id: 3,
                file_path: "mid.md".to_string(),
                heading_context: "# Mid".to_string(),
                content: "mid".to_string(),
                line_start: 5,
                embedding: Embedding::new(vec![0.7, 0.7]),
            },
        ];

        let hits = top_k(&query, &chunks, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].file_path, "high.md");
        assert_eq!(hits[1].file_path, "mid.md");
        assert!(hits[0].score > hits[1].score);
    }
}

