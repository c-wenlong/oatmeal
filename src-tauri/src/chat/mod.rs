//! Asking questions across meetings.
//!
//! The same discipline as the summary panels, for the same reason: an answer
//! that cites a line nobody said is worse than no answer, because it looks like
//! evidence. Every citation is checked against the retrieved context and dropped
//! if it does not resolve, and a claim that loses all its citations is marked
//! uncited rather than deleted — the user can still judge it, and silently
//! removing text the model produced hides what it did.
//!
//! Retrieval is the search built for G24, scoped to one meeting or one folder.
//! That is deliberate: two ways to find a line would eventually disagree, and
//! the one people already trust is the one they can see working.

pub mod prompt;

use serde::{Deserialize, Serialize};

use crate::db::repo;
use crate::llm::keys::KeyStore;
use crate::llm::provider::ProviderConfig;
use crate::llm::provider::{ChatRequest, Message};
use crate::llm::{LlmClient, LlmError};

#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error(transparent)]
    Db(#[from] crate::db::DbError),
    #[error("nothing in this scope to answer from")]
    NothingToAnswer,
    #[error("the model did not return usable JSON: {0}")]
    Malformed(String),
}

/// One line of transcript offered to the model as evidence.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextLine {
    pub utterance_id: i64,
    pub meeting_id: String,
    pub meeting_title: Option<String>,
    pub start_ms: i64,
    pub text: String,
}

/// A citation the model attached to a claim, after validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub utterance_id: i64,
    #[serde(default)]
    pub meeting_id: String,
    #[serde(default)]
    pub meeting_title: Option<String>,
    #[serde(default)]
    pub start_ms: i64,
}

/// One claim in an answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub text: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
}

impl Claim {
    pub fn is_cited(&self) -> bool {
        !self.citations.is_empty()
    }
}

/// A complete answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub claims: Vec<Claim>,
}

/// What validation removed. Surfaced so a model that invents citations is
/// visible rather than quietly cleaned up after.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerReport {
    pub dropped_citations: usize,
    pub uncited_claims: usize,
}

impl AnswerReport {
    pub fn is_clean(&self) -> bool {
        self.dropped_citations == 0
    }
}

/// Parses the model's reply, tolerating the wrappers models add.
pub fn parse(raw: &str) -> Result<Answer, ChatError> {
    let trimmed = raw.trim();
    // Models fence JSON in markdown even when told not to, and sometimes
    // preface it with a sentence. Both are recoverable; refusing them would
    // fail on output that is otherwise perfectly good.
    let body = strip_fence(trimmed);
    let candidate = match (body.find('{'), body.rfind('}')) {
        (Some(start), Some(end)) if end > start => &body[start..=end],
        _ => body,
    };

    serde_json::from_str(candidate).map_err(|e| ChatError::Malformed(e.to_string()))
}

fn strip_fence(text: &str) -> &str {
    let without_open = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .unwrap_or(text);
    without_open
        .strip_suffix("```")
        .unwrap_or(without_open)
        .trim()
}

/// Drops citations that do not name a line actually in the context.
///
/// The anti-hallucination gate. A model asked to cite will invent plausible
/// ids — Phase 3 caught a live model doing it twice — and a citation chip that
/// jumps nowhere is worse than an honest "uncited", because the user has no
/// reason to doubt it until they click.
pub fn validate(answer: &mut Answer, context: &[ContextLine]) -> AnswerReport {
    let mut report = AnswerReport::default();

    for claim in &mut answer.claims {
        let before = claim.citations.len();
        claim.citations.retain_mut(|citation| {
            match context
                .iter()
                .find(|line| line.utterance_id == citation.utterance_id)
            {
                Some(line) => {
                    // Fill in from the context rather than trusting the model's
                    // idea of which meeting a line belongs to.
                    citation.meeting_id = line.meeting_id.clone();
                    citation.meeting_title = line.meeting_title.clone();
                    citation.start_ms = line.start_ms;
                    true
                }
                None => false,
            }
        });
        claim
            .citations
            .dedup_by_key(|citation| citation.utterance_id);
        report.dropped_citations += before - claim.citations.len();
    }

    // A claim with no text at all is not a claim.
    answer.claims.retain(|claim| !claim.text.trim().is_empty());
    report.uncited_claims = answer.claims.iter().filter(|c| !c.is_cited()).count();
    report
}

/// The answer plus what validation had to remove.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub answer: Answer,
    pub report: AnswerReport,
    /// The lines the answer was allowed to draw on, so the UI can show them.
    pub context: Vec<ContextLine>,
}

/// Answers a question over a meeting or a folder.
pub async fn ask(
    client: &LlmClient,
    config: &ProviderConfig,
    store: &dyn KeyStore,
    question: &str,
    context: Vec<ContextLine>,
) -> Result<ChatReply, ChatError> {
    if context.is_empty() {
        // Answering with no evidence produces confident fiction, every time.
        return Err(ChatError::NothingToAnswer);
    }

    let request = ChatRequest::new(vec![
        Message::system(prompt::system_prompt()),
        Message::user(prompt::build_user_prompt(question, &context)),
    ])
    .with_schema(prompt::schema());

    let raw = client.chat(config, &request, store).await?;
    let mut answer = parse(&raw)?;
    let report = validate(&mut answer, &context);

    Ok(ChatReply {
        answer,
        report,
        context,
    })
}

/// Builds the evidence set for a question.
///
/// Uses the G24 search so the model sees the same lines the user would have
/// found by searching — which makes a wrong answer debuggable rather than
/// mysterious.
pub async fn gather_context(
    conn: &rusqlite::Connection,
    question: &str,
    meeting_id: Option<&str>,
    folder_id: Option<&str>,
    embedder: &impl crate::embed::Embedder,
    limit: usize,
) -> Result<Vec<ContextLine>, crate::db::DbError> {
    // A single meeting is small enough to hand over whole. Retrieval over one
    // conversation risks dropping the line that answers the question because
    // the question happened not to share its words.
    if let Some(meeting_id) = meeting_id {
        let utterances = repo::meeting_utterances(conn, meeting_id)?;
        let headers = repo::meeting_headers(conn, &[meeting_id.to_string()])?;
        let title = headers.get(meeting_id).and_then(|(t, _)| t.clone());
        return Ok(utterances
            .into_iter()
            .map(|u| ContextLine {
                utterance_id: u.id,
                meeting_id: meeting_id.to_string(),
                meeting_title: title.clone(),
                start_ms: u.start_ms,
                text: u.text,
            })
            .collect());
    }

    let response = crate::search::search(conn, question, folder_id, embedder, limit).await?;
    let mut lines = Vec::new();
    for result in response.results {
        for hit in result.meeting.hits {
            lines.push(ContextLine {
                utterance_id: hit.utterance_id,
                meeting_id: hit.meeting_id,
                meeting_title: result.meeting.title.clone(),
                start_ms: hit.start_ms,
                text: hit.text,
            });
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: i64, meeting: &str, text: &str) -> ContextLine {
        ContextLine {
            utterance_id: id,
            meeting_id: meeting.into(),
            meeting_title: Some("Standup".into()),
            start_ms: id * 1_000,
            text: text.into(),
        }
    }

    fn answer(claims: &[(&str, &[i64])]) -> Answer {
        Answer {
            claims: claims
                .iter()
                .map(|(text, ids)| Claim {
                    text: (*text).to_string(),
                    citations: ids
                        .iter()
                        .map(|id| Citation {
                            utterance_id: *id,
                            meeting_id: String::new(),
                            meeting_title: None,
                            start_ms: 0,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_real_citation_survives_and_is_filled_in() {
        let context = vec![line(7, "m1", "we ship on thursday")];
        let mut out = answer(&[("Shipping Thursday", &[7])]);
        let report = validate(&mut out, &context);

        assert!(report.is_clean());
        let citation = &out.claims[0].citations[0];
        assert_eq!(citation.meeting_id, "m1");
        assert_eq!(citation.start_ms, 7_000);
        assert_eq!(citation.meeting_title.as_deref(), Some("Standup"));
    }

    #[test]
    fn an_invented_citation_is_dropped() {
        // The failure this module exists to prevent. A live model did exactly
        // this twice during Phase 3.
        let context = vec![line(7, "m1", "we ship on thursday")];
        let mut out = answer(&[("Shipping Thursday", &[999])]);
        let report = validate(&mut out, &context);

        assert_eq!(report.dropped_citations, 1);
        assert!(out.claims[0].citations.is_empty());
    }

    #[test]
    fn a_claim_that_loses_every_citation_is_kept_and_marked() {
        // Deleting it would hide what the model produced; the user can still
        // judge an uncited sentence.
        let context = vec![line(7, "m1", "we ship on thursday")];
        let mut out = answer(&[("Something invented", &[404])]);
        let report = validate(&mut out, &context);

        assert_eq!(out.claims.len(), 1);
        assert!(!out.claims[0].is_cited());
        assert_eq!(report.uncited_claims, 1);
    }

    #[test]
    fn a_partly_invented_claim_keeps_its_real_citation() {
        let context = vec![line(7, "m1", "a"), line(8, "m1", "b")];
        let mut out = answer(&[("Mixed", &[7, 999, 8])]);
        let report = validate(&mut out, &context);

        assert_eq!(report.dropped_citations, 1);
        assert_eq!(out.claims[0].citations.len(), 2);
    }

    #[test]
    fn a_duplicated_citation_is_collapsed() {
        // Two chips pointing at the same line is noise, not evidence.
        let context = vec![line(7, "m1", "a")];
        let mut out = answer(&[("Repeated", &[7, 7])]);
        validate(&mut out, &context);
        assert_eq!(out.claims[0].citations.len(), 1);
    }

    #[test]
    fn an_empty_claim_is_removed() {
        let context = vec![line(7, "m1", "a")];
        let mut out = answer(&[("   ", &[7]), ("Real", &[7])]);
        validate(&mut out, &context);
        assert_eq!(out.claims.len(), 1);
        assert_eq!(out.claims[0].text, "Real");
    }

    #[test]
    fn citations_across_meetings_each_resolve_to_their_own() {
        // The folder case: an answer spanning five calls must not attribute a
        // line to the wrong one.
        let context = vec![line(1, "m1", "a"), line(2, "m2", "b")];
        let mut out = answer(&[("Across calls", &[1, 2])]);
        validate(&mut out, &context);

        assert_eq!(out.claims[0].citations[0].meeting_id, "m1");
        assert_eq!(out.claims[0].citations[1].meeting_id, "m2");
    }

    #[test]
    fn plain_json_parses() {
        let parsed =
            parse(r#"{"claims":[{"text":"hi","citations":[{"utteranceId":3}]}]}"#).unwrap();
        assert_eq!(parsed.claims[0].citations[0].utterance_id, 3);
    }

    #[test]
    fn a_fenced_reply_parses() {
        // Models fence JSON even when told not to.
        let parsed = parse("```json\n{\"claims\":[{\"text\":\"hi\"}]}\n```").unwrap();
        assert_eq!(parsed.claims[0].text, "hi");
    }

    #[test]
    fn a_reply_with_a_preamble_parses() {
        let parsed = parse("Sure! Here is the answer:\n{\"claims\":[{\"text\":\"hi\"}]}").unwrap();
        assert_eq!(parsed.claims.len(), 1);
    }

    #[test]
    fn a_claim_with_no_citations_field_parses_as_uncited() {
        let parsed = parse(r#"{"claims":[{"text":"hi"}]}"#).unwrap();
        assert!(!parsed.claims[0].is_cited());
    }

    #[test]
    fn nonsense_is_an_error_not_a_panic() {
        assert!(parse("not json at all").is_err());
        assert!(parse("").is_err());
    }

    /// G25's done-when, against a real model.
    ///
    /// Ignored by default — CI has no model. Run it when touching the prompt or
    /// the validator; it is the only thing that catches a model that has started
    /// inventing citations, which is exactly what happened in Phase 3.
    ///
    ///   cargo test --lib live_a_folder_question -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_a_folder_question_is_answered_with_resolving_citations() {
        use crate::db::Database;
        use crate::embed::HashEmbedder;
        use crate::llm::keys::MemoryKeyStore;
        use crate::llm::provider::{ProviderConfig, ProviderKind};

        let db = Database::open_in_memory().unwrap();
        let conn = db.connection();

        // Five short meetings with one commitment each.
        let calls = [
            (
                "m1",
                "Vendor review",
                "we will sign the vendor contract by friday",
            ),
            ("m2", "Standup", "I will own the rollback plan myself"),
            (
                "m3",
                "Planning",
                "we agreed to cut the release scope in half",
            ),
            (
                "m4",
                "Retro",
                "we will add a second reviewer to every deploy",
            ),
            (
                "m5",
                "Budget",
                "we committed to freezing hiring until the next quarter",
            ),
        ];
        for (index, (id, title, line)) in calls.iter().enumerate() {
            repo::insert_meeting(conn, id, title, index as i64).unwrap();
            let utterance =
                repo::insert_utterance(conn, id, 0, "system", line, 1_000, 4_000, None).unwrap();
            repo::replace_embedding(
                conn,
                "utterance",
                &utterance.to_string(),
                &HashEmbedder::vector(line),
            )
            .unwrap();
        }

        let question = "what did we commit to across these calls?";
        let context = gather_context(conn, question, None, None, &HashEmbedder, 20)
            .await
            .unwrap();
        assert!(
            !context.is_empty(),
            "retrieval found nothing to answer from"
        );
        eprintln!("retrieved {} lines", context.len());

        let model =
            std::env::var("OATMEAL_PROVIDER_MODEL").unwrap_or_else(|_| "gemma4:e2b".to_string());
        let mut config = ProviderConfig::preset(ProviderKind::Ollama);
        config.model = model.clone();

        let reply = ask(
            &LlmClient::new(),
            &config,
            &MemoryKeyStore::default(),
            question,
            context.clone(),
        )
        .await
        .expect("the model did not answer");

        eprintln!("model: {model}");
        for claim in &reply.answer.claims {
            eprintln!(
                "  - {} [{}]",
                claim.text,
                claim
                    .citations
                    .iter()
                    .map(|c| format!("#{}", c.utterance_id))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        eprintln!(
            "dropped {} invented citation(s), {} uncited claim(s)",
            reply.report.dropped_citations, reply.report.uncited_claims
        );

        assert!(!reply.answer.claims.is_empty(), "no claims came back");

        // The done-when: every citation that survived resolves to a real line.
        for claim in &reply.answer.claims {
            for citation in &claim.citations {
                assert!(
                    context
                        .iter()
                        .any(|line| line.utterance_id == citation.utterance_id),
                    "citation #{} does not resolve",
                    citation.utterance_id
                );
                assert!(!citation.meeting_id.is_empty(), "citation lost its meeting");
            }
        }
    }
}
