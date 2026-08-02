//! Turning text into vectors.
//!
//! Behind a trait for two reasons: the linker's logic must be testable without a
//! model running, and the embedding backend is genuinely likely to change —
//! today it is `nomic-embed-text` through whatever OpenAI-shaped server is
//! configured, tomorrow it could be CoreML in the sidecar.

use serde::Deserialize;

use crate::db::EMBEDDING_DIM;

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("could not reach the embedding model at {url}: {detail}")]
    Unreachable { url: String, detail: String },
    #[error("the embedding model returned {actual} dimensions, expected {expected}")]
    WrongDimension { expected: usize, actual: usize },
    #[error("unexpected response from the embedding model: {0}")]
    Malformed(String),
}

#[allow(async_fn_in_trait)]
pub trait Embedder: Send + Sync {
    /// Embeds a batch. Order of the result matches the input.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

/// Cosine similarity of two unit-ish vectors, clamped to 0..=1.
///
/// Clamped because the linker combines this with a temporal score on the same
/// scale — a negative similarity would otherwise let an unrelated line drag a
/// combined score below one with no evidence at all.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a.sqrt() * norm_b.sqrt())).clamp(0.0, 1.0)
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
}

/// Talks to any OpenAI-shaped `/embeddings` endpoint — Ollama, LM Studio, or a
/// local `llama-server`.
pub struct HttpEmbedder {
    http: reqwest::Client,
    base_url: String,
    model: String,
}

impl HttpEmbedder {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into(),
            model: model.into(),
        }
    }

    /// Default: whatever local server is already running.
    pub fn local() -> Self {
        Self::new("http://localhost:11434/v1", "nomic-embed-text:v1.5")
    }

    fn url(&self) -> String {
        format!("{}/embeddings", self.base_url.trim_end_matches('/'))
    }
}

impl Embedder for HttpEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = self.url();
        let response = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "model": self.model, "input": texts }))
            .send()
            .await
            .map_err(|e| EmbedError::Unreachable {
                url: url.clone(),
                detail: e.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(EmbedError::Malformed(format!(
                "{status}: {}",
                body.chars().take(200).collect::<String>()
            )));
        }

        let parsed: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| EmbedError::Malformed(e.to_string()))?;

        if parsed.data.len() != texts.len() {
            return Err(EmbedError::Malformed(format!(
                "asked for {} embeddings, got {}",
                texts.len(),
                parsed.data.len()
            )));
        }

        for item in &parsed.data {
            // A width mismatch would otherwise fail much later, inside sqlite-vec,
            // with an error that says nothing about which model produced it.
            if item.embedding.len() != EMBEDDING_DIM {
                return Err(EmbedError::WrongDimension {
                    expected: EMBEDDING_DIM,
                    actual: item.embedding.len(),
                });
            }
        }

        Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
    }
}

/// Deterministic embedder for tests.
///
/// Bag-of-words hashing: texts sharing words land near each other, which is
/// enough to exercise the linker's semantic layer without a model. Nothing here
/// claims to be a good embedding — it is a stand-in with the one property the
/// tests depend on.
pub struct HashEmbedder;

impl HashEmbedder {
    pub fn vector(text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; EMBEDDING_DIM];
        for word in text
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            let mut hash: u64 = 1469598103934665603;
            for byte in word.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(1099511628211);
            }
            v[(hash as usize) % EMBEDDING_DIM] += 1.0;
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

impl Embedder for HashEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| Self::vector(t)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors_are_maximally_similar() {
        let v = HashEmbedder::vector("the deadline is Thursday");
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn shared_wording_scores_higher_than_unrelated_text() {
        // The one property the linker's semantic layer depends on.
        let note = HashEmbedder::vector("deadline migration");
        let related = HashEmbedder::vector("the deadline for the migration is Thursday");
        let unrelated = HashEmbedder::vector("who is bringing lunch tomorrow");

        assert!(
            cosine(&note, &related) > cosine(&note, &unrelated),
            "related text did not score higher than unrelated"
        );
    }

    #[test]
    fn cosine_is_never_negative() {
        // The linker adds this to a temporal score on the same scale; a negative
        // would let an unrelated line pull a combined score down with no evidence.
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        assert_eq!(cosine(&a, &b), 0.0);
    }

    #[test]
    fn mismatched_or_empty_vectors_score_zero_rather_than_panicking() {
        assert_eq!(cosine(&[1.0, 2.0], &[1.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn embeddings_match_the_schema_width() {
        // A mismatch fails inside sqlite-vec with an error that names neither
        // the model nor the column.
        assert_eq!(HashEmbedder::vector("anything").len(), EMBEDDING_DIM);
    }

    #[test]
    fn embedding_is_deterministic() {
        assert_eq!(
            HashEmbedder::vector("the deadline"),
            HashEmbedder::vector("the deadline")
        );
    }

    #[test]
    fn casing_and_punctuation_do_not_change_the_vector() {
        // ASR punctuates inconsistently; notes are typed casually.
        assert_eq!(
            HashEmbedder::vector("Deadline, Thursday!"),
            HashEmbedder::vector("deadline thursday")
        );
    }

    #[tokio::test]
    async fn an_empty_batch_makes_no_request() {
        let embedder = HttpEmbedder::new("http://127.0.0.1:1/v1", "nope");
        assert!(embedder.embed(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unreachable_embedder_says_where_it_tried() {
        let embedder = HttpEmbedder::new("http://127.0.0.1:1/v1", "nope");
        let err = embedder.embed(&["hello".to_string()]).await.unwrap_err();
        assert!(matches!(err, EmbedError::Unreachable { .. }));
        assert!(err.to_string().contains("127.0.0.1:1"));
    }

    #[test]
    fn the_endpoint_tolerates_a_trailing_slash() {
        assert_eq!(
            HttpEmbedder::new("http://localhost:11434/v1/", "m").url(),
            "http://localhost:11434/v1/embeddings"
        );
    }
}
