//! The one provider that doesn't speak OpenAI's shape.
//!
//! Two differences matter: Anthropic takes the system prompt as a top-level
//! `system` field rather than a message with `role: "system"`, and it returns
//! `content` as a list of typed blocks rather than a single string.
//!
//! There is no JSON-schema response format, so structured output is requested
//! in the prompt instead — which is exactly why nothing downstream trusts the
//! shape of what comes back (G14 validates every citation regardless).

use super::provider::{ChatRequest, Message, ProviderConfig, Role};
use super::LlmError;

pub fn build_body(config: &ProviderConfig, request: &ChatRequest) -> serde_json::Value {
    // Anthropic rejects `role: "system"` inside `messages`; it belongs at the
    // top level. Several system messages are joined rather than dropped.
    let system: Vec<&str> = request
        .messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(|m| m.content.as_str())
        .collect();

    let conversation: Vec<&Message> = request
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .collect();

    let mut body = serde_json::json!({
        "model": config.model,
        "max_tokens": request.max_tokens,
        "temperature": request.temperature,
        "messages": conversation,
    });

    if !system.is_empty() {
        body["system"] = serde_json::Value::String(system.join("\n\n"));
    }

    if let Some(schema) = &request.json_schema {
        // No `response_format` equivalent, so the instruction goes in `system`.
        let instruction = format!(
            "Respond with JSON only — no prose, no markdown fence — matching this schema:\n{}",
            serde_json::to_string(schema).unwrap_or_default()
        );
        body["system"] = serde_json::Value::String(match body.get("system") {
            Some(serde_json::Value::String(existing)) => {
                format!("{existing}\n\n{instruction}")
            }
            _ => instruction,
        });
    }

    body
}

pub fn extract_text(body: &serde_json::Value) -> Result<String, LlmError> {
    let blocks = body
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| LlmError::Malformed {
            detail: "no content array in Anthropic response".into(),
        })?;

    // Content is a list of typed blocks; only the text ones are the answer.
    let text: String = blocks
        .iter()
        .filter(|block| block.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("");

    if text.is_empty() {
        return Err(LlmError::Malformed {
            detail: "Anthropic response contained no text blocks".into(),
        });
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::{ProviderKind, Role};

    fn config() -> ProviderConfig {
        ProviderConfig::preset(ProviderKind::Anthropic)
    }

    #[test]
    fn the_system_prompt_moves_out_of_messages() {
        // Anthropic rejects `role: "system"` inside `messages` outright.
        let request = ChatRequest::new(vec![
            Message::system("you summarise meetings"),
            Message::user("summarise this"),
        ]);
        let body = build_body(&config(), &request);

        assert_eq!(body["system"], "you summarise meetings");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn several_system_messages_are_joined_not_dropped() {
        let request = ChatRequest::new(vec![
            Message::system("first"),
            Message::system("second"),
            Message::user("go"),
        ]);
        let body = build_body(&config(), &request);
        assert!(body["system"].as_str().unwrap().contains("first"));
        assert!(body["system"].as_str().unwrap().contains("second"));
    }

    #[test]
    fn a_request_with_no_system_prompt_omits_the_field() {
        let body = build_body(&config(), &ChatRequest::new(vec![Message::user("hi")]));
        assert!(body.get("system").is_none());
    }

    #[test]
    fn a_schema_becomes_a_prompt_instruction() {
        // There is no response_format here, so the schema has to be asked for.
        let schema = serde_json::json!({"type": "object"});
        let body = build_body(&config(), &ChatRequest::new(vec![]).with_schema(schema));
        let system = body["system"].as_str().unwrap();
        assert!(system.contains("JSON"));
        assert!(system.contains("\"type\":\"object\""));
    }

    #[test]
    fn the_schema_instruction_does_not_clobber_the_system_prompt() {
        let request = ChatRequest::new(vec![Message::system("you summarise meetings")])
            .with_schema(serde_json::json!({"type": "object"}));
        let body = build_body(&config(), &request);
        let system = body["system"].as_str().unwrap();
        assert!(system.contains("you summarise meetings"));
        assert!(system.contains("JSON"));
    }

    #[test]
    fn extracts_text_blocks() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "the summary"}]
        });
        assert_eq!(extract_text(&body).unwrap(), "the summary");
    }

    #[test]
    fn joins_several_text_blocks_and_ignores_other_kinds() {
        let body = serde_json::json!({
            "content": [
                {"type": "text", "text": "part one "},
                {"type": "thinking", "thinking": "ignored"},
                {"type": "text", "text": "part two"}
            ]
        });
        assert_eq!(extract_text(&body).unwrap(), "part one part two");
    }

    #[test]
    fn a_response_with_no_text_is_an_error() {
        // A blank panel would look like the model had nothing to say.
        let body = serde_json::json!({"content": [{"type": "thinking", "thinking": "x"}]});
        assert!(extract_text(&body).is_err());
        assert!(extract_text(&serde_json::json!({})).is_err());
    }

    #[test]
    fn roles_serialise_lowercase_as_the_api_expects() {
        assert_eq!(serde_json::to_value(Role::User).unwrap(), "user");
        assert_eq!(serde_json::to_value(Role::Assistant).unwrap(), "assistant");
    }
}
