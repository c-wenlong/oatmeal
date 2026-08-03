//! Generating a panel from a meeting.
//!
//! A panel is a regenerable view over an immutable transcript (SPEC section 8).
//! Nothing here may touch `utterances` or `note_blocks`.

pub mod content;
pub mod prompt;

use std::collections::HashSet;

use crate::db::repo::{NoteBlock, Utterance};
use crate::llm::provider::{ChatRequest, Message, ProviderConfig};
use crate::llm::{keys::KeyStore, LlmClient, LlmError};

pub use content::{PanelContent, ValidationReport};
pub use prompt::Template;

#[derive(Debug, thiserror::Error)]
pub enum PanelError {
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error("the model did not return a usable panel: {0}")]
    Unusable(String),
    #[error("this meeting has no transcript to summarise yet")]
    NothingToSummarise,
}

/// A generated panel plus what the citation gate rejected.
#[derive(Debug, Clone)]
pub struct Generated {
    pub content: PanelContent,
    pub report: ValidationReport,
    pub provider: String,
    pub model: String,
}

/// Generates a panel and validates every citation it claims.
///
/// The retry exists because local models routinely ignore `response_format` and
/// answer with prose. One reminder usually fixes it; more than one means the
/// model cannot do the task, and looping would just burn time.
pub async fn generate(
    client: &LlmClient,
    config: &ProviderConfig,
    store: &dyn KeyStore,
    template: &Template,
    utterances: &[Utterance],
    notes: &[NoteBlock],
) -> Result<Generated, PanelError> {
    if utterances.is_empty() {
        // Summarising nothing produces confident fiction, every time.
        return Err(PanelError::NothingToSummarise);
    }

    let user = prompt::build_user_prompt(template, utterances, notes);
    let request = ChatRequest::new(vec![
        Message::system(prompt::system_prompt()),
        Message::user(user.clone()),
    ])
    .with_schema(content::schema());

    let raw = client.chat(config, &request, store).await?;

    let parsed = match content::parse(&raw) {
        Ok(parsed) => parsed,
        Err(first_error) => {
            // Repair pass: hand the model its own output and ask for JSON only.
            let repair = ChatRequest::new(vec![
                Message::system(prompt::system_prompt()),
                Message::user(user),
                Message::system(format!(
                    "That response could not be parsed ({first_error}). \
                     Return the same content as JSON only — no prose, no markdown fence."
                )),
            ])
            .with_schema(content::schema());

            let retried = client.chat(config, &repair, store).await?;
            content::parse(&retried).map_err(PanelError::Unusable)?
        }
    };

    let valid_utterances: HashSet<i64> = utterances.iter().map(|u| u.id).collect();
    let valid_notes: HashSet<String> = notes.iter().map(|n| n.block_id.clone()).collect();
    let (validated, report) = content::validate(parsed, &valid_utterances, &valid_notes);

    Ok(Generated {
        content: validated,
        report,
        // The enum name, not the label: this is read back by the privacy panel
        // to classify local versus cloud, and a display string is a lossy key.
        provider: format!("{:?}", config.kind).to_lowercase(),
        model: config.model.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::keys::MemoryKeyStore;
    use crate::llm::provider::ProviderKind;

    #[tokio::test]
    async fn a_meeting_with_no_transcript_is_refused_before_any_request() {
        // Asking a model to summarise nothing reliably produces fiction, and it
        // would cost a paid API call to get it.
        let err = generate(
            &LlmClient::new(),
            &ProviderConfig::preset(ProviderKind::Ollama),
            &MemoryKeyStore::default(),
            &prompt::builtin_templates()[0],
            &[],
            &[],
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PanelError::NothingToSummarise));
    }
}
