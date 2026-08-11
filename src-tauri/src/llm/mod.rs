//! Talking to language models.
//!
//! Split so the parts that are hard to test over a network aren't the parts
//! that carry the logic:
//! - `provider` — presets and request/response shaping, pure
//! - `anthropic` — the one adapter, pure
//! - `keys`     — Keychain behind a trait, so tests use memory
//! - this file  — the only place that performs I/O

pub mod anthropic;
pub mod bundled;
pub mod download;
pub mod keys;
pub mod ollama;
pub mod provider;

use std::time::Duration;

use provider::{ChatRequest, ProviderConfig, ProviderKind};

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("{provider} needs an API key — add one in settings")]
    MissingKey { provider: String },
    #[error("could not reach {url}: {detail}")]
    Unreachable { url: String, detail: String },
    #[error("{provider} rejected the request ({status}): {detail}")]
    Rejected {
        provider: String,
        status: u16,
        detail: String,
    },
    #[error("unexpected response: {detail}")]
    Malformed { detail: String },
    #[error("keychain: {0}")]
    Key(#[from] keys::KeyError),
}

impl LlmError {
    /// Whether retrying the same request could plausibly work.
    ///
    /// Drives the repair-retry in G14: a rate limit is worth a second attempt,
    /// a missing key never is.
    pub fn is_retryable(&self) -> bool {
        match self {
            LlmError::Unreachable { .. } => true,
            LlmError::Rejected { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }
}

/// Maps an HTTP failure onto something a user can act on.
///
/// The status codes matter more than the body: a 401 means "your key is wrong",
/// and saying so beats echoing a provider's JSON error at someone.
pub fn classify(provider: ProviderKind, status: u16, body: &str) -> LlmError {
    let detail = match status {
        401 | 403 => "the API key was rejected".to_string(),
        404 => "no such model, or the base URL is wrong".to_string(),
        429 => "rate limited".to_string(),
        s if s >= 500 => "the provider is having trouble".to_string(),
        _ => {
            // Providers nest their message differently; try the common shapes
            // before falling back to the raw body.
            serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .or_else(|| v.get("message"))
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body.chars().take(200).collect())
        }
    };

    LlmError::Rejected {
        provider: provider.label().to_string(),
        status,
        detail,
    }
}

pub struct LlmClient {
    http: reqwest::Client,
}

impl Default for LlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                // Long enough for a slow local model on a big transcript,
                // short enough that a wedged server doesn't hang the app.
                .timeout(Duration::from_secs(180))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Sends a chat request and returns the assistant's text.
    pub async fn chat(
        &self,
        config: &ProviderConfig,
        request: &ChatRequest,
        store: &dyn keys::KeyStore,
    ) -> Result<String, LlmError> {
        let url = config.chat_url();
        let body = provider::build_body(config, request);

        let mut builder = self.http.post(&url).json(&body);

        if config.kind.requires_key() {
            let reference = config
                .keychain_ref
                .as_deref()
                .ok_or_else(|| LlmError::MissingKey {
                    provider: config.kind.label().to_string(),
                })?;
            let key = store
                .get(reference)?
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| LlmError::MissingKey {
                    provider: config.kind.label().to_string(),
                })?;

            builder = match config.kind {
                // Anthropic uses its own header scheme rather than Bearer.
                ProviderKind::Anthropic => builder
                    .header("x-api-key", key)
                    .header("anthropic-version", "2023-06-01"),
                _ => builder.bearer_auth(key),
            };
        }

        let response = builder.send().await.map_err(|e| LlmError::Unreachable {
            url: url.clone(),
            detail: e.to_string(),
        })?;

        let status = response.status();
        let text = response.text().await.map_err(|e| LlmError::Unreachable {
            url,
            detail: e.to_string(),
        })?;

        if !status.is_success() {
            return Err(classify(config.kind, status.as_u16(), &text));
        }

        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| LlmError::Malformed {
                detail: format!("response was not JSON: {e}"),
            })?;

        provider::extract_text(config.kind, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keys::KeyStore;

    #[test]
    fn a_bad_key_says_so_rather_than_echoing_json() {
        let err = classify(ProviderKind::Openai, 401, r#"{"error":{"message":"blah"}}"#);
        assert!(err.to_string().contains("API key was rejected"));
    }

    #[test]
    fn a_404_points_at_the_model_or_url() {
        // The two things a user can actually fix.
        let err = classify(ProviderKind::Ollama, 404, "");
        assert!(err.to_string().contains("model") || err.to_string().contains("URL"));
    }

    #[test]
    fn an_unknown_status_surfaces_the_provider_message() {
        let err = classify(
            ProviderKind::Openai,
            400,
            r#"{"error":{"message":"context length exceeded"}}"#,
        );
        assert!(err.to_string().contains("context length exceeded"));
    }

    #[test]
    fn an_unparseable_error_body_still_says_something() {
        let err = classify(ProviderKind::Openai, 400, "plain text failure");
        assert!(err.to_string().contains("plain text failure"));
    }

    #[test]
    fn a_giant_error_body_is_truncated() {
        // Providers occasionally return an entire HTML page.
        let err = classify(ProviderKind::Openai, 400, &"x".repeat(10_000));
        assert!(err.to_string().len() < 400);
    }

    #[test]
    fn rate_limits_and_server_errors_are_worth_retrying() {
        assert!(classify(ProviderKind::Openai, 429, "").is_retryable());
        assert!(classify(ProviderKind::Openai, 503, "").is_retryable());
        assert!(LlmError::Unreachable {
            url: "x".into(),
            detail: "timeout".into()
        }
        .is_retryable());
    }

    #[test]
    fn a_missing_key_is_never_retried() {
        // Retrying cannot conjure a key, and hammering a 401 looks like abuse.
        assert!(!LlmError::MissingKey {
            provider: "OpenAI".into()
        }
        .is_retryable());
        assert!(!classify(ProviderKind::Openai, 401, "").is_retryable());
        assert!(!classify(ProviderKind::Openai, 400, "").is_retryable());
    }

    #[tokio::test]
    async fn a_cloud_provider_without_a_key_fails_before_any_request() {
        // Nothing should leave the machine when we already know it will 401.
        let config = ProviderConfig::preset(ProviderKind::Openai);
        let store = keys::MemoryKeyStore::default();
        let err = LlmClient::new()
            .chat(&config, &ChatRequest::new(vec![]), &store)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::MissingKey { .. }));
    }

    #[tokio::test]
    async fn a_blank_key_counts_as_missing() {
        // Pasting an empty string into settings should not become a 401 later.
        let config = ProviderConfig::preset(ProviderKind::Openai);
        let store = keys::MemoryKeyStore::default();
        store.set("openai", "   ").unwrap();
        let err = LlmClient::new()
            .chat(&config, &ChatRequest::new(vec![]), &store)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::MissingKey { .. }));
    }

    #[tokio::test]
    async fn an_unreachable_local_server_is_reported_as_unreachable() {
        let mut config = ProviderConfig::preset(ProviderKind::Ollama);
        // Port 1 is reserved and never listening.
        config.base_url = "http://127.0.0.1:1/v1".into();
        let store = keys::MemoryKeyStore::default();

        let err = LlmClient::new()
            .chat(&config, &ChatRequest::new(vec![]), &store)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Unreachable { .. }), "got {err:?}");
        assert!(err.is_retryable());
    }
}
