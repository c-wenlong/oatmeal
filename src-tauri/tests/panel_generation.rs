//! End-to-end panel generation against a real HTTP endpoint.
//!
//! The first test is hermetic: a tiny stub server returns a response containing
//! a citation to an utterance that does not exist, and the panel that comes back
//! must not contain it. That is G14's whole reason for existing — a chip that
//! scrolls nowhere looks like evidence — so it is proved against the real
//! request path rather than by calling `validate` directly.
//!
//! The second talks to Ollama if it happens to be running, and skips otherwise.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use oatmeal_lib::db::repo::{NoteBlock, Utterance};
use oatmeal_lib::llm::keys::MemoryKeyStore;
use oatmeal_lib::llm::provider::{ProviderConfig, ProviderKind};
use oatmeal_lib::llm::LlmClient;
use oatmeal_lib::panel::{self, prompt::builtin_templates};

fn utterance(id: i64, source: &str, text: &str) -> Utterance {
    Utterance {
        id,
        seq: id,
        source: source.into(),
        text: text.into(),
        start_ms: id * 1000,
        end_ms: id * 1000 + 900,
        confidence: None,
    }
}

fn note(block_id: &str, text: &str) -> NoteBlock {
    NoteBlock {
        block_id: block_id.into(),
        seq: 0,
        text: text.into(),
        first_typed_at_ms: Some(1_500),
        last_edited_at_ms: Some(1_500),
    }
}

/// Serves one canned chat-completions response, then exits.
fn stub_server(body: serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve_one(stream, &body);
        }
    });

    format!("http://127.0.0.1:{port}/v1")
}

fn serve_one(mut stream: TcpStream, body: &serde_json::Value) {
    // Read past the headers so the client's write completes.
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
    }
    if content_length > 0 {
        let mut request_body = vec![0u8; content_length];
        use std::io::Read;
        let _ = reader.read_exact(&mut request_body);
    }

    let payload = serde_json::json!({
        "choices": [{ "message": { "role": "assistant", "content": body.to_string() } }]
    })
    .to_string();

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        payload.len(),
        payload
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

#[tokio::test]
async fn an_invented_citation_never_reaches_the_panel() {
    // The model cites #12 (real), #9999 (invented), and note b-nope (invented).
    let model_output = serde_json::json!({
        "sections": [{
            "heading": "Decisions",
            "bullets": [
                { "text": "Ship on Thursday", "sourceUtterances": [12, 9999], "fromNote": "b1" },
                { "text": "Entirely invented", "sourceUtterances": [4242], "fromNote": "b-nope" }
            ]
        }]
    });

    let mut config = ProviderConfig::preset(ProviderKind::Ollama);
    config.base_url = stub_server(model_output);

    let utterances = vec![utterance(12, "system", "let's ship Thursday")];
    let notes = vec![note("b1", "ship date")];

    let generated = panel::generate(
        &LlmClient::new(),
        &config,
        &MemoryKeyStore::default(),
        &builtin_templates()[0],
        &utterances,
        &notes,
    )
    .await
    .expect("generation should succeed");

    let bullets = &generated.content.sections[0].bullets;

    // The real citation survives; the invented one is gone.
    assert_eq!(bullets[0].source_utterances, vec![12]);
    assert_eq!(bullets[0].from_note.as_deref(), Some("b1"));

    // A bullet whose every citation was invented keeps its text but loses the
    // citations, and is reported as uncited rather than deleted.
    assert!(bullets[1].source_utterances.is_empty());
    assert_eq!(bullets[1].from_note, None);
    assert_eq!(bullets[1].text, "Entirely invented");

    assert_eq!(generated.report.dropped_utterances, 2);
    assert_eq!(generated.report.dropped_notes, 1);
    assert!(generated.report.had_hallucinations());

    // Nothing in the finished panel points at a line that does not exist.
    for section in &generated.content.sections {
        for bullet in &section.bullets {
            for id in &bullet.source_utterances {
                assert!(
                    utterances.iter().any(|u| u.id == *id),
                    "panel cites #{id}, which is not in the transcript"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_model_answering_with_prose_is_reported_not_silently_empty() {
    // Local models do this. An empty panel would look like a meeting where
    // nothing was said.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        // Two connections: the first attempt and the repair retry.
        for _ in 0..2 {
            if let Ok((mut stream, _)) = listener.accept() {
                let payload = serde_json::json!({
                    "choices": [{ "message": { "content": "I'm sorry, I can't do that." } }]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                let _ = stream.write_all(response.as_bytes());
            }
        }
    });

    let mut config = ProviderConfig::preset(ProviderKind::Ollama);
    config.base_url = format!("http://127.0.0.1:{port}/v1");

    let result = panel::generate(
        &LlmClient::new(),
        &config,
        &MemoryKeyStore::default(),
        &builtin_templates()[0],
        &[utterance(1, "mic", "hello")],
        &[],
    )
    .await;

    assert!(result.is_err(), "prose was accepted as a panel");
}

/// Live check against Ollama. Skips when nothing is serving, so CI stays green
/// without a model.
#[tokio::test]
async fn ollama_produces_a_panel_whose_citations_all_resolve() {
    let probe = reqwest::Client::new()
        .get("http://localhost:11434/api/tags")
        .timeout(std::time::Duration::from_millis(700))
        .send()
        .await;
    let Ok(response) = probe else {
        eprintln!("skipping: no Ollama on localhost:11434");
        return;
    };
    if !response.status().is_success() {
        eprintln!("skipping: Ollama did not answer");
        return;
    }

    let model = std::env::var("OATMEAL_TEST_MODEL").unwrap_or_else(|_| "gemma4:e2b".into());
    let mut config = ProviderConfig::preset(ProviderKind::Ollama);
    config.model = model;

    let utterances = vec![
        utterance(
            1,
            "system",
            "So the deadline for the migration is the fourteenth.",
        ),
        utterance(2, "mic", "Got it, I'll own the rollback plan."),
        utterance(3, "system", "Perfect, let's review it on Thursday."),
    ];
    let notes = vec![note("b1", "deadline = 14th")];

    let generated = match panel::generate(
        &LlmClient::new(),
        &config,
        &MemoryKeyStore::default(),
        &builtin_templates()[0],
        &utterances,
        &notes,
    )
    .await
    {
        Ok(generated) => generated,
        Err(err) => {
            // A missing model is an environment gap, not a defect under test.
            eprintln!("skipping: Ollama could not generate ({err})");
            return;
        }
    };

    assert!(
        generated.content.bullet_count() > 0,
        "the model returned an empty panel"
    );

    // Whatever the model claimed, nothing unresolvable survives the gate.
    for section in &generated.content.sections {
        for bullet in &section.bullets {
            for id in &bullet.source_utterances {
                assert!(
                    utterances.iter().any(|u| u.id == *id),
                    "panel cites #{id}, which is not in the transcript"
                );
            }
            if let Some(note_id) = &bullet.from_note {
                assert!(notes.iter().any(|n| &n.block_id == note_id));
            }
        }
    }

    eprintln!(
        "ollama panel: {} bullets, {} invented citations dropped",
        generated.content.bullet_count(),
        generated.report.dropped_utterances
    );
}
