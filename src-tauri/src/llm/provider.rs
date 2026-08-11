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
            // Measured against ten real meetings — see docs/perf.md. It cited
            // every bullet of every summary, at 1.7 GB resident and roughly
            // half the latency of the 9.5 GB `gemma4:latest`, which cited no
            // better. The previous default, `llama3.2`, was never measured
            // here and is not a model this project has evidence for.
            ProviderKind::Ollama => "gemma4:e2b",
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
    /// Recovers a kind from whatever was stored against a generation.
    ///
    /// `panels.provider` holds the *display label* ("LM Studio"), not the enum
    /// name — so the privacy panel cannot classify a generation by string
    /// matching in the frontend, and an earlier attempt to do so marked every
    /// local summary as cloud. Both forms are accepted because rows written
    /// before this existed carry the label.
    pub fn from_stored(value: &str) -> Option<Self> {
        let normalised = value.trim().to_lowercase().replace([' ', '_', '-'], "");
        Self::all().iter().copied().find(|kind| {
            let label = kind.label().to_lowercase().replace([' ', '_', '-'], "");
            let name = format!("{kind:?}").to_lowercase();
            normalised == name || normalised == label || label.starts_with(&normalised)
        })
    }

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
            // Ollama's own API rather than its OpenAI-compatible one. The
            // compatible endpoint silently truncates any prompt over the
            // server's default 4096-token window and cannot be told otherwise
            // — `options` is not part of that wire format and is ignored. A
            // 42-minute meeting arrives as a fraction of itself and comes back
            // as a confident summary of the fraction. See `num_ctx`.
            ProviderKind::Ollama => format!("{}/api/chat", base.trim_end_matches("/v1")),
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

/// Ollama's default context window, which is not nearly enough for a meeting.
pub const OLLAMA_DEFAULT_NUM_CTX: usize = 4096;

/// The largest window worth asking for.
///
/// The KV cache is allocated for whatever is requested, so asking for the
/// maximum on every call would cost gigabytes to summarise a two-minute
/// standup.
pub const OLLAMA_MAX_NUM_CTX: usize = 32_768;

/// A context window that will actually hold the prompt.
///
/// Ollama truncates rather than failing when a prompt exceeds the window — it
/// keeps what fits and answers from that, so an over-long transcript produces
/// a plausible summary of a sliver with citations that still resolve. There is
/// no error to catch; the only defence is asking for a window big enough.
///
/// Sized from the characters actually being sent, at a deliberately pessimistic
/// 3 characters per token — under-estimating brings back exactly the silent
/// truncation this exists to prevent — plus room for the reply, rounded up to a
/// power of two because that is how the cache is allocated anyway.
pub fn num_ctx(prompt_chars: usize, reply_tokens: usize) -> usize {
    let needed = prompt_chars / 3 + reply_tokens;
    let mut window = OLLAMA_DEFAULT_NUM_CTX;
    while window < needed && window < OLLAMA_MAX_NUM_CTX {
        window *= 2;
    }
    window.min(OLLAMA_MAX_NUM_CTX)
}

/// Builds the JSON body for a provider. Pure, so every provider's wire format
/// is testable without a network.
pub fn build_body(config: &ProviderConfig, request: &ChatRequest) -> serde_json::Value {
    match config.kind {
        ProviderKind::Anthropic => super::anthropic::build_body(config, request),
        ProviderKind::Ollama => {
            let chars: usize = request.messages.iter().map(|m| m.content.len()).sum();
            let mut body = serde_json::json!({
                "model": config.model,
                "messages": request.messages,
                "stream": false,
                "options": {
                    "temperature": request.temperature,
                    "num_ctx": num_ctx(chars, request.max_tokens as usize),
                },
            });
            // Ollama takes the schema itself here, not a `response_format`
            // wrapper, and constrains generation to it.
            if let Some(schema) = &request.json_schema {
                body["format"] = schema.clone();
            }
            body
        }
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
        ProviderKind::Ollama => body
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .ok_or_else(|| super::LlmError::Malformed {
                detail: "no message.content in response".into(),
            }),
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
    fn the_local_default_is_the_model_we_measured() {
        // Changing this is a claim about quality, so it should fail loudly and
        // send whoever changed it to docs/perf.md for the evidence.
        assert_eq!(
            ProviderConfig::preset(ProviderKind::Ollama).model,
            "gemma4:e2b"
        );
    }

    #[test]
    fn a_short_prompt_keeps_the_default_window() {
        // Asking for the maximum every time allocates a KV cache measured in
        // gigabytes to summarise a two-minute standup.
        assert_eq!(num_ctx(1_000, 1_024), OLLAMA_DEFAULT_NUM_CTX);
    }

    #[test]
    fn a_long_meeting_gets_a_window_that_holds_it() {
        // The 42-minute meeting that exposed this: ~43,600 characters, about
        // 12,800 tokens. At the default it arrived as 659 of them.
        let window = num_ctx(43_600, 1_024);
        assert!(
            window >= 43_600 / 3 + 1_024,
            "{window} would still truncate"
        );
        assert!(window.is_power_of_two());
    }

    #[test]
    fn the_window_never_exceeds_the_ceiling() {
        // A four-hour recording should ask for a lot and then stop asking.
        assert_eq!(num_ctx(10_000_000, 1_024), OLLAMA_MAX_NUM_CTX);
    }

    #[test]
    fn ollama_talks_to_its_own_api_not_the_compatible_one() {
        // `/v1/chat/completions` ignores `options`, so the window cannot be set
        // and the prompt is silently cut to 4096 tokens.
        let mut config = ProviderConfig::preset(ProviderKind::Ollama);
        assert_eq!(config.chat_url(), "http://localhost:11434/api/chat");
        // And a user who typed the base URL with a trailing slash gets the same.
        config.base_url = "http://localhost:11434/v1/".into();
        assert_eq!(config.chat_url(), "http://localhost:11434/api/chat");
    }

    #[test]
    fn the_ollama_body_carries_the_window_and_the_schema() {
        let config = ProviderConfig::preset(ProviderKind::Ollama);
        let long = "x".repeat(40_000);
        let schema = serde_json::json!({"type": "object"});
        let body = build_body(
            &config,
            &ChatRequest::new(vec![Message::user(long)]).with_schema(schema.clone()),
        );
        assert!(body["options"]["num_ctx"].as_u64().unwrap() > OLLAMA_DEFAULT_NUM_CTX as u64);
        // Ollama takes the schema bare, not wrapped in a response_format.
        assert_eq!(body["format"], schema);
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn ollama_replies_are_read_from_its_own_shape() {
        // Native responses have no `choices`; reading the OpenAI shape here
        // would report every successful call as malformed.
        let body = serde_json::json!({"message": {"content": "hello"}});
        assert_eq!(extract_text(ProviderKind::Ollama, &body).unwrap(), "hello");
    }

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
        // `//chat/completions` and a baffling 404. Checked on LM Studio, which
        // is still on the OpenAI-compatible path — Ollama has its own test
        // above, because it no longer uses that path at all.
        let mut config = ProviderConfig::preset(ProviderKind::Lmstudio);
        config.base_url = "http://localhost:1234/v1/".into();
        assert_eq!(
            config.chat_url(),
            "http://localhost:1234/v1/chat/completions"
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

#[cfg(test)]
mod live {
    use super::*;

    /// The shipped default, against a running Ollama, with nothing overridden.
    ///
    /// `cargo test --lib llm::provider::live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "needs a running Ollama"]
    async fn the_default_local_provider_answers_out_of_the_box() {
        // No model override, no base-url edit: exactly what a new install has.
        let config = ProviderConfig::preset(ProviderKind::Ollama);
        let request = ChatRequest::new(vec![Message::user(
            "Reply with the single word: ready".to_string(),
        )]);
        let response: serde_json::Value = reqwest::Client::new()
            .post(config.chat_url())
            .json(&build_body(&config, &request))
            .send()
            .await
            .expect("ollama unreachable")
            .json()
            .await
            .expect("json");
        // A model that is not pulled comes back as an error object, not a
        // message — which is exactly the failure a new user would hit.
        assert!(
            response.get("error").is_none(),
            "default model {} is not usable: {response}",
            config.model
        );
        let text = extract_text(ProviderKind::Ollama, &response).expect("content");
        eprintln!("model={} reply={}", config.model, text.trim());
        assert!(text.to_lowercase().contains("ready"), "unexpected: {text}");
    }

    /// The 42-minute meeting, end to end against a running Ollama.
    ///
    /// `cargo test --lib llm::provider::live -- --ignored --nocapture`
    /// Needs Ollama up with the model pulled, and the transcript path in
    /// `OATMEAL_LIVE_TRANSCRIPT`.
    #[tokio::test]
    #[ignore = "needs a running Ollama"]
    async fn a_long_meeting_is_not_silently_truncated() {
        let Ok(path) = std::env::var("OATMEAL_LIVE_TRANSCRIPT") else {
            eprintln!("set OATMEAL_LIVE_TRANSCRIPT");
            return;
        };
        let text = std::fs::read_to_string(path).expect("transcript");
        // A passphrase at the very front. If the window is too small Ollama
        // keeps the tail, so this is the first thing to disappear.
        let prompt = format!(
            "The passphrase is ZEBRAFISH-11.\n\n{text}\n\nReply with JSON \
             {{\"passphrase\":\"...\"}} giving the passphrase stated above."
        );
        let mut config = ProviderConfig::preset(ProviderKind::Ollama);
        config.model = std::env::var("OATMEAL_PROVIDER_MODEL").unwrap_or("gemma4:e2b".into());

        let request = ChatRequest::new(vec![Message::user(prompt)]);
        let body = build_body(&config, &request);
        let window = body["options"]["num_ctx"].as_u64().unwrap();
        assert!(
            window > OLLAMA_DEFAULT_NUM_CTX as u64,
            "window {window} too small"
        );

        let response: serde_json::Value = reqwest::Client::new()
            .post(config.chat_url())
            .json(&body)
            .send()
            .await
            .expect("ollama unreachable")
            .json()
            .await
            .expect("json");
        let text = extract_text(ProviderKind::Ollama, &response).expect("content");
        eprintln!("window={window} reply={text}");
        assert!(
            text.contains("ZEBRAFISH-11"),
            "the front of the prompt was dropped: {text}"
        );
    }
}
