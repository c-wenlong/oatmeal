//! The local model Oatmeal can run by itself.
//!
//! JIT-downloaded rather than shipped in the app bundle (SPEC section 10):
//! `llama-server` plus a model is hundreds of megabytes, most users will point
//! at a cloud provider or an existing Ollama, and decoupling it means llama.cpp
//! updates don't need an app release.
//!
//! This is what makes the fully-local path real rather than aspirational — a
//! user with no API key and no Ollama install can still summarise, offline,
//! after one guided download.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::Serialize;

/// Where the runtime lives, under the app support directory.
pub const RUNTIME_DIR: &str = "runtime";
pub const SERVER_BINARY: &str = "llama-server";
pub const PORT: u16 = 8080;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RuntimeState {
    /// Neither the server nor a model is present.
    NotInstalled,
    /// The server is there but no model has been downloaded.
    NeedsModel,
    Ready,
    Running {
        pid: u32,
    },
}

/// A model the bundled runtime can fetch.
///
/// Deliberately a short, curated list: "pick any GGUF from Hugging Face" is a
/// research project, not a setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub url: String,
    pub filename: String,
    pub approx_bytes: u64,
    pub note: String,
}

pub fn model_options() -> Vec<ModelOption> {
    vec![
        ModelOption {
            id: "qwen2.5-3b".into(),
            name: "Qwen2.5 3B Instruct".into(),
            url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-3b-instruct-q4_k_m.gguf".into(),
            approx_bytes: 2_100_000_000,
            note: "Fast. Good enough for short meetings.".into(),
        },
        ModelOption {
            id: "qwen2.5-7b".into(),
            name: "Qwen2.5 7B Instruct".into(),
            url: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-7b-instruct-q4_k_m.gguf".into(),
            approx_bytes: 4_700_000_000,
            note: "Better structure and citation discipline. Slower.".into(),
        },
    ]
}

/// The llama.cpp release asset for this machine.
///
/// Only Apple Silicon is supported (SPEC section 2), so there is one answer
/// rather than a platform matrix.
pub fn server_asset_url(release_tag: &str) -> String {
    format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{release_tag}/llama-{release_tag}-bin-macos-arm64.zip"
    )
}

pub struct Runtime {
    root: PathBuf,
    child: Mutex<Option<Child>>,
}

impl Runtime {
    pub fn new(app_support: &Path) -> Self {
        Self {
            root: app_support.join(RUNTIME_DIR),
            child: Mutex::new(None),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn server_path(&self) -> PathBuf {
        self.root.join(SERVER_BINARY)
    }

    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn model_path(&self, filename: &str) -> PathBuf {
        self.models_dir().join(filename)
    }

    /// A model counts as installed only if it is plausibly complete.
    ///
    /// An interrupted download leaves a short file that `llama-server` accepts
    /// and then fails on mid-generation — the same class of bug the ASR model
    /// check exists for (see docs/audio-findings.md).
    pub fn installed_models(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.models_dir()) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .metadata()
                    .map(|m| m.is_file() && m.len() > 100_000_000)
                    .unwrap_or(false)
            })
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".gguf"))
            .collect()
    }

    pub fn state(&self) -> RuntimeState {
        if let Some(child) = self
            .child
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.id()))
        {
            return RuntimeState::Running { pid: child };
        }
        if !self.server_path().is_file() {
            return RuntimeState::NotInstalled;
        }
        if self.installed_models().is_empty() {
            return RuntimeState::NeedsModel;
        }
        RuntimeState::Ready
    }

    /// Starts `llama-server` against the first installed model.
    pub fn start(&self) -> Result<u32, String> {
        if let RuntimeState::Running { pid } = self.state() {
            return Ok(pid);
        }

        let server = self.server_path();
        if !server.is_file() {
            return Err("the local runtime is not installed yet".into());
        }
        let model = self
            .installed_models()
            .into_iter()
            .next()
            .ok_or("no local model has been downloaded yet")?;

        let child = Command::new(&server)
            .arg("--model")
            .arg(self.model_path(&model))
            .arg("--port")
            .arg(PORT.to_string())
            // Bind to loopback explicitly: the whole point of this path is that
            // nothing leaves the machine, and a default of 0.0.0.0 would expose
            // an unauthenticated model server to the local network.
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--ctx-size")
            .arg("16384")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start the local model server: {e}"))?;

        let pid = child.id();
        *self.child.lock().map_err(|_| "runtime lock poisoned")? = Some(child);
        Ok(pid)
    }

    pub fn stop(&self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_runtime() -> (tempfile::TempDir, Runtime) {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(dir.path());
        (dir, runtime)
    }

    #[test]
    fn a_fresh_machine_reports_nothing_installed() {
        let (_dir, runtime) = temp_runtime();
        assert_eq!(runtime.state(), RuntimeState::NotInstalled);
    }

    #[test]
    fn a_server_with_no_model_reports_needs_model() {
        // Starting here would fail with a llama.cpp usage error rather than
        // something a user can act on.
        let (_dir, runtime) = temp_runtime();
        std::fs::create_dir_all(runtime.root()).unwrap();
        std::fs::write(runtime.server_path(), b"#!/bin/sh\n").unwrap();

        assert_eq!(runtime.state(), RuntimeState::NeedsModel);
    }

    #[test]
    fn a_server_and_model_together_are_ready() {
        let (_dir, runtime) = temp_runtime();
        std::fs::create_dir_all(runtime.models_dir()).unwrap();
        std::fs::write(runtime.server_path(), b"#!/bin/sh\n").unwrap();
        std::fs::write(runtime.model_path("m.gguf"), vec![0u8; 150_000_000]).unwrap();

        assert_eq!(runtime.state(), RuntimeState::Ready);
    }

    #[test]
    fn a_truncated_model_does_not_count_as_installed() {
        // An interrupted download otherwise reports Ready and then fails
        // mid-generation — the exact failure mode the ASR model check exists
        // for (docs/audio-findings.md).
        let (_dir, runtime) = temp_runtime();
        std::fs::create_dir_all(runtime.models_dir()).unwrap();
        std::fs::write(runtime.server_path(), b"#!/bin/sh\n").unwrap();
        std::fs::write(runtime.model_path("partial.gguf"), b"not a real model").unwrap();

        assert!(runtime.installed_models().is_empty());
        assert_eq!(runtime.state(), RuntimeState::NeedsModel);
    }

    #[test]
    fn non_gguf_files_are_ignored() {
        let (_dir, runtime) = temp_runtime();
        std::fs::create_dir_all(runtime.models_dir()).unwrap();
        std::fs::write(runtime.model_path("README.txt"), vec![0u8; 150_000_000]).unwrap();
        assert!(runtime.installed_models().is_empty());
    }

    #[test]
    fn starting_without_an_install_explains_itself() {
        let (_dir, runtime) = temp_runtime();
        let err = runtime.start().unwrap_err();
        assert!(err.contains("not installed"), "unhelpful error: {err}");
    }

    #[test]
    fn starting_without_a_model_explains_itself() {
        let (_dir, runtime) = temp_runtime();
        std::fs::create_dir_all(runtime.root()).unwrap();
        std::fs::write(runtime.server_path(), b"#!/bin/sh\n").unwrap();

        let err = runtime.start().unwrap_err();
        assert!(err.contains("model"), "unhelpful error: {err}");
    }

    #[test]
    fn every_model_option_is_downloadable() {
        for option in model_options() {
            assert!(
                option.url.starts_with("https://"),
                "{} is not https",
                option.id
            );
            assert!(option.filename.ends_with(".gguf"));
            assert!(option.approx_bytes > 0);
            assert!(!option.note.is_empty(), "{} has no guidance", option.id);
        }
    }

    #[test]
    fn model_option_ids_are_unique() {
        let options = model_options();
        let ids: std::collections::HashSet<_> = options.iter().map(|o| &o.id).collect();
        assert_eq!(ids.len(), options.len());
    }

    #[test]
    fn the_server_asset_targets_apple_silicon() {
        let url = server_asset_url("b4000");
        assert!(url.contains("macos-arm64"), "wrong platform: {url}");
        assert!(url.contains("b4000"));
        assert!(url.starts_with("https://"));
    }

    #[test]
    fn the_bundled_preset_points_at_the_port_we_serve_on() {
        // A mismatch here fails only at generation time with a connection error.
        use crate::llm::provider::{ProviderConfig, ProviderKind};
        let config = ProviderConfig::preset(ProviderKind::Bundled);
        assert!(
            config.base_url.contains(&PORT.to_string()),
            "preset {} does not match the port the runtime binds ({PORT})",
            config.base_url
        );
    }

    #[test]
    fn stopping_a_runtime_that_never_started_is_harmless() {
        let (_dir, runtime) = temp_runtime();
        runtime.stop();
        assert_eq!(runtime.state(), RuntimeState::NotInstalled);
    }
}
