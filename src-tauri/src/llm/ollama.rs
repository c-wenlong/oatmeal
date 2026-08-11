//! Is the chosen Ollama model actually there, and can we fetch it if not.
//!
//! Ollama will happily accept a model it does not have and answer with a 404
//! buried in a JSON body, which reaches the user as an opaque provider error.
//! The default model shipped for months without being pulled on the machine it
//! was developed on precisely because nothing ever asked this question.

use serde::{Deserialize, Serialize};

/// What is known about the configured model.
///
/// Three states rather than a boolean: "Ollama is not running" and "the model
/// is not installed" need completely different things from the user, and
/// collapsing them produces a Download button that cannot possibly work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelAvailability {
    /// Nothing is listening. Ollama is not installed, or not started.
    Unreachable {
        detail: String,
    },
    /// Ollama answered and does not have this model.
    Missing {
        model: String,
    },
    Installed {
        model: String,
    },
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TaggedModel>,
}

#[derive(Debug, Deserialize)]
struct TaggedModel {
    name: String,
}

/// Whether a configured name refers to an installed one.
///
/// Ollama stores everything tagged, so `gemma4` on disk is `gemma4:latest`.
/// A user who types the short form has not chosen a different model, and
/// telling them it is missing — next to a Download button that would fetch
/// what they already have — is worse than useless.
pub fn names_match(configured: &str, installed: &str) -> bool {
    let normalise = |name: &str| {
        let name = name.trim();
        match name.split_once(':') {
            Some(_) => name.to_string(),
            None => format!("{name}:latest"),
        }
    };
    normalise(configured) == normalise(installed)
}

/// The `/api/tags` URL for a configured base, which may point at either API.
pub fn tags_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/').trim_end_matches("/v1");
    format!("{base}/api/tags")
}

pub fn pull_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/').trim_end_matches("/v1");
    format!("{base}/api/pull")
}

/// Reads the installed list and decides what the model's state is.
pub fn availability_from(model: &str, body: &str) -> ModelAvailability {
    let installed = match serde_json::from_str::<TagsResponse>(body) {
        Ok(tags) => tags.models,
        Err(err) => {
            return ModelAvailability::Unreachable {
                detail: format!("could not read the model list: {err}"),
            }
        }
    };
    if installed.iter().any(|m| names_match(model, &m.name)) {
        ModelAvailability::Installed {
            model: model.to_string(),
        }
    } else {
        ModelAvailability::Missing {
            model: model.to_string(),
        }
    }
}

/// One line of `/api/pull`'s streaming response.
#[derive(Debug, Deserialize)]
pub struct PullLine {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub completed: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Turns a pull line into the progress shape the UI already renders.
///
/// Ollama reports its phases — manifest, several layers, verify — as separate
/// runs of bytes, so `completed` restarts at zero repeatedly. Reporting each
/// run honestly is better than a single bar that jumps backwards.
pub fn progress_from(line: &PullLine, id: &str) -> super::download::DownloadProgress {
    super::download::DownloadProgress {
        id: id.to_string(),
        downloaded: line.completed.unwrap_or(0),
        total: line.total,
        // Ollama's last line is `success`; everything before it is a phase.
        done: line.status == "success",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_matches_its_latest_tag() {
        // What is on disk is `gemma4:latest`; what the user typed is `gemma4`.
        // Calling that missing offers to download what they already have.
        assert!(names_match("gemma4", "gemma4:latest"));
        assert!(names_match("gemma4:latest", "gemma4"));
        assert!(names_match("gemma4:e2b", "gemma4:e2b"));
    }

    #[test]
    fn different_tags_are_different_models() {
        // `e2b` is 1.7 GB and `latest` is 9.5 GB. Treating them as one would
        // report a model as installed and then fail on the first request.
        assert!(!names_match("gemma4:e2b", "gemma4:latest"));
        assert!(!names_match("gemma4:e2b", "llama3.2:latest"));
    }

    #[test]
    fn whitespace_does_not_make_a_new_model() {
        assert!(names_match(" gemma4:e2b ", "gemma4:e2b"));
    }

    #[test]
    fn an_installed_model_is_recognised() {
        let body = r#"{"models":[{"name":"gemma4:e2b"},{"name":"nomic-embed-text:v1.5"}]}"#;
        assert_eq!(
            availability_from("gemma4:e2b", body),
            ModelAvailability::Installed {
                model: "gemma4:e2b".into()
            }
        );
    }

    #[test]
    fn a_missing_model_is_named_so_it_can_be_offered() {
        let body = r#"{"models":[{"name":"llama3.2:latest"}]}"#;
        assert_eq!(
            availability_from("gemma4:e2b", body),
            ModelAvailability::Missing {
                model: "gemma4:e2b".into()
            }
        );
    }

    #[test]
    fn an_empty_install_is_missing_rather_than_broken() {
        // A fresh Ollama with nothing pulled answers `{"models":[]}`. That is
        // the most common first-run state and it is not an error.
        assert_eq!(
            availability_from("gemma4:e2b", r#"{"models":[]}"#),
            ModelAvailability::Missing {
                model: "gemma4:e2b".into()
            }
        );
    }

    #[test]
    fn a_reply_that_is_not_a_model_list_is_not_a_missing_model() {
        // Something is listening on the port but it is not Ollama. Offering to
        // download would produce a second confusing failure.
        let state = availability_from("gemma4:e2b", "<html>hello</html>");
        assert!(matches!(state, ModelAvailability::Unreachable { .. }));
    }

    #[test]
    fn urls_are_built_from_either_base() {
        // The preset points at the native API; a user may have pasted the
        // OpenAI-compatible one from Ollama's own docs.
        assert_eq!(
            tags_url("http://localhost:11434"),
            "http://localhost:11434/api/tags"
        );
        assert_eq!(
            tags_url("http://localhost:11434/v1"),
            "http://localhost:11434/api/tags"
        );
        assert_eq!(
            tags_url("http://localhost:11434/v1/"),
            "http://localhost:11434/api/tags"
        );
        assert_eq!(
            pull_url("http://localhost:11434/v1"),
            "http://localhost:11434/api/pull"
        );
    }

    #[test]
    fn only_the_success_line_finishes_the_download() {
        let pulling = PullLine {
            status: "pulling manifest".into(),
            total: Some(100),
            completed: Some(40),
            error: None,
        };
        assert!(!progress_from(&pulling, "m").done);
        let finished = PullLine {
            status: "success".into(),
            total: None,
            completed: None,
            error: None,
        };
        assert!(progress_from(&finished, "m").done);
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Against a running Ollama: the check must agree with reality.
    ///
    /// `cargo test --lib llm::ollama::live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "needs a running Ollama"]
    async fn the_check_agrees_with_what_is_actually_installed() {
        let base = "http://localhost:11434";
        let body = reqwest::get(tags_url(base))
            .await
            .expect("ollama unreachable")
            .text()
            .await
            .expect("body");

        // The shipped default must read as installed on a machine that has it.
        let default_model = crate::llm::provider::ProviderKind::Ollama.default_model();
        eprintln!(
            "{default_model} -> {:?}",
            availability_from(default_model, &body)
        );

        // And a model nobody has must read as missing, not as unreachable —
        // that is the difference between offering a download and blaming the
        // server.
        let state = availability_from("definitely-not-a-real-model:v9", &body);
        assert!(
            matches!(state, ModelAvailability::Missing { .. }),
            "expected Missing, got {state:?}"
        );
    }
}
