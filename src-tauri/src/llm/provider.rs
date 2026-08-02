//! Provider presets and the request shape they all speak.
//!
//! Everything is modelled on OpenAI's chat-completions API because Ollama,
//! LM Studio, `llama-server` and OpenRouter already speak it. That leaves
//! Anthropic as the single adapter (see `anthropic.rs`) rather than five
//! bespoke clients, and makes "add a provider" a row in a table.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Openrouter,
    Ollama,
    Lmstudio,
    /// The `llama-server` Oatmeal downloads and runs itself (G13).
    Bundled,
}

impl ProviderKind {
    pub fn all() -> &'static [ProviderKind] {
        &[
            ProviderKind::Anthropic,
            ProviderKind::Openai,
            ProviderKind::Openrouter,
            ProviderKind::Ollama,
            ProviderKind::Lmstudio,
            ProviderKind::Bundled,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "Anthropic",
            ProviderKind::Openai => "OpenAI",
            ProviderKind::Openrouter => "OpenRouter",
            ProviderKind::Ollama => "Ollama",
            ProviderKind::Lmstudio => "LM Studio",
            ProviderKind::Bundled => "Bundled (llama.cpp)",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "https://api.anthropic.com",
            ProviderKind::Openai => "https://api.openai.com/v1",
            ProviderKind::Openrouter => "https://openrouter.ai/api/v1",
            ProviderKind::Ollama => "http://localhost:11434/v1",
            ProviderKind::Lmstudio => "http://localhost:1234/v1",
            ProviderKind::Bundled => "http://localhost:8080/v1",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "claude-sonnet-5",
            ProviderKind::Openai => "gpt-5",
            ProviderKind::Openrouter => "anthropic/claude-sonnet-5",
            ProviderKind::Ollama => "llama3.2",
            ProviderKind::Lmstudio => "local-model",
            ProviderKind::Bundled => "local",
        }
    }

    /// Whether the provider needs a key at all.
    ///
    /// Drives the UI: asking for a key to talk to `localhost` is noise, and
    /// silently accepting a missing key for a cloud provider produces a 401
    /// much later.
    pub fn requires_key(self) -> bool {
        matches!(
            self,
            ProviderKind::Anthropic | ProviderKind::Openai | ProviderKind::Openrouter
        )
    }

    /// True when requests never leave the machine.
    ///
    /// Surfaced per generation so the privacy panel (G27) can report what
    /// actually happened rather than what the app was configured to do.
    pub fn is_local(self) -> bool {
        matches!(
            self,
            ProviderKind::Ollama | ProviderKind::Lmstudio | ProviderKind::Bundled
        )
    }
}

/// A configured provider. Note there is no key field: keys live in the
/// Keychain and are fetched at request time by `keychain_ref`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub keychain_ref: Option<String>,
}

impl ProviderConfig {
    pub fn preset(kind: ProviderKind) -> Self {
        Self {
            id: format!("{kind:?}").to_lowercase(),
            kind,
            base_url: kind.default_base_url().to_string(),
            model: kind.default_model().to_string(),
            keychain_ref: if kind.requires_key() {
                Some(format!("{kind:?}").to_lowercase())
            } else {
                None
            },
        }
    }

    /// Full URL for a chat completion.
    ///
    /// Trailing slashes on a user-edited base URL are the classic way to end up
    /// requesting `//chat/completions` and getting a confusing 404.
    pub fn chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        match self.kind {
            ProviderKind::Anthropic => format!("{base}/v1/messages"),
            _ => format!("{base}/chat/completions"),
        }
    }
}

// ------------------------------------------------------------------ messages

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: u32,
    /// When set, the model is asked to return JSON matching this schema.
    /// Providers that ignore it are caught by validation downstream (G14).
    pub json_schema: Option<serde_json::Value>,
}

impl ChatRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            // Summarisation should be reproducible, not creative.
            temperature: 0.2,
            max_tokens: 4096,
            json_schema: None,
        }
    }

    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.json_schema = Some(schema);
        self
    }
}

/// Builds the JSON body for a provider. Pure, so every provider's wire format
/// is testable without a network.
pub fn build_body(config: &ProviderConfig, request: &ChatRequest) -> serde_json::Value {
    match config.kind {
        ProviderKind::Anthropic => super::anthropic::build_body(config, request),
        _ => {
            let mut body = serde_json::json!({
                "model": config.model,
                "messages": request.messages,
                "temperature": request.temperature,
                "max_tokens": request.max_tokens,
            });

            if let Some(schema) = &request.json_schema {
                // The `json_schema` response format is what OpenAI and
                // OpenRouter honour. Local servers usually ignore it, which is
                // why nothing downstream trusts the output's shape.
                body["response_format"] = serde_json::json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "oatmeal_panel",
                        "strict": true,
                        "schema": schema,
                    }
                });
            }
            body
        }
    }
}

/// Extracts the assistant's text from a provider response.
pub fn extract_text(
    kind: ProviderKind,
    body: &serde_json::Value,
) -> Result<String, super::LlmError> {
    match kind {
        ProviderKind::Anthropic => super::anthropic::extract_text(body),
        _ => body
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .ok_or_else(|| super::LlmError::Malformed {
                detail: "no choices[0].message.content in response".into(),
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_a_base_url_and_model() {
        for kind in ProviderKind::all() {
            let config = ProviderConfig::preset(*kind);
            assert!(!config.base_url.is_empty(), "{kind:?} has no base url");
            assert!(!config.model.is_empty(), "{kind:?} has no model");
        }
    }

    #[test]
    fn local_providers_never_ask_for_a_key() {
        // Asking for a key to talk to localhost is noise the user cannot satisfy.
        for kind in ProviderKind::all().iter().filter(|k| k.is_local()) {
            assert!(
                !kind.requires_key(),
                "{kind:?} asks for a key it cannot use"
            );
            assert!(ProviderConfig::preset(*kind).keychain_ref.is_none());
        }
    }

    #[test]
    fn cloud_providers_all_require_a_key() {
        for kind in ProviderKind::all().iter().filter(|k| !k.is_local()) {
            assert!(kind.requires_key(), "{kind:?} would 401 with no key");
            assert!(ProviderConfig::preset(*kind).keychain_ref.is_some());
        }
    }

    #[test]
    fn local_providers_point_at_localhost() {
        // The privacy claim rests on this: "local" must actually mean local.
        for kind in ProviderKind::all().iter().filter(|k| k.is_local()) {
            let url = kind.default_base_url();
            assert!(
                url.contains("localhost") || url.contains("127.0.0.1"),
                "{kind:?} claims to be local but points at {url}"
            );
        }
    }

    #[test]
    fn cloud_providers_use_https() {
        for kind in ProviderKind::all().iter().filter(|k| !k.is_local()) {
            assert!(
                kind.default_base_url().starts_with("https://"),
                "{kind:?} would send an API key in the clear"
            );
        }
    }

    #[test]
    fn chat_url_tolerates_a_trailing_slash() {
        // A user pasting a base URL with a trailing slash otherwise produces
        // `//chat/completions` and a baffling 404.
        let mut config = ProviderConfig::preset(ProviderKind::Ollama);
        config.base_url = "http://localhost:11434/v1/".into();
        assert_eq!(
            config.chat_url(),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn anthropic_uses_its_own_endpoint() {
        let config = ProviderConfig::preset(ProviderKind::Anthropic);
        assert_eq!(config.chat_url(), "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn openai_shaped_body_carries_model_and_messages() {
        let config = ProviderConfig::preset(ProviderKind::Openai);
        let request = ChatRequest::new(vec![Message::system("be brief"), Message::user("hi")]);
        let body = build_body(&config, &request);

        assert_eq!(body["model"], config.model);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hi");
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn a_schema_becomes_a_strict_response_format() {
        let config = ProviderConfig::preset(ProviderKind::Openai);
        let schema = serde_json::json!({"type": "object"});
        let body = build_body(
            &config,
            &ChatRequest::new(vec![]).with_schema(schema.clone()),
        );

        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
    }

    #[test]
    fn temperature_defaults_low_for_reproducible_summaries() {
        assert!(ChatRequest::new(vec![]).temperature <= 0.3);
    }

    #[test]
    fn extracts_text_from_an_openai_response() {
        let body = serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": "the summary"}}]
        });
        assert_eq!(
            extract_text(ProviderKind::Openai, &body).unwrap(),
            "the summary"
        );
    }

    #[test]
    fn a_response_with_no_content_is_an_error_not_an_empty_summary() {
        // Returning "" here would render a blank panel that looks like the model
        // simply had nothing to say.
        let body = serde_json::json!({"choices": []});
        assert!(extract_text(ProviderKind::Openai, &body).is_err());
    }
}
