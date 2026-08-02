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

use super::download;

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
            approx_bytes: 2_104_932_768,
            note: "Fast. Good enough for short meetings.".into(),
        },
        ModelOption {
            id: "qwen2.5-7b".into(),
            name: "Qwen2.5 7B Instruct".into(),
            // Not the official Qwen repo: it publishes q4_k_m only as a
            // two-shard split, and a single file is worth more here than
            // matching the upstream name. Verified 200, 4.68 GB.
            url: "https://huggingface.co/bartowski/Qwen2.5-7B-Instruct-GGUF/resolve/main/Qwen2.5-7B-Instruct-Q4_K_M.gguf".into(),
            filename: "Qwen2.5-7B-Instruct-Q4_K_M.gguf".into(),
            approx_bytes: 4_683_074_240,
            note: "Better structure and citation discipline. Slower.".into(),
        },
    ]
}

/// The llama.cpp release this fetches.
///
/// Pinned rather than resolved from the "latest" API: a release the developer
/// has actually verified beats whatever landed upstream this morning, and the
/// asset naming has changed before — see below. Bump it deliberately.
pub const SERVER_RELEASE: &str = "b10229";

/// The llama.cpp release asset for this machine.
///
/// Only Apple Silicon is supported (SPEC section 2), so there is one answer
/// rather than a platform matrix.
///
/// **`.tar.gz`, not `.zip`.** llama.cpp used to publish macOS builds as zips
/// and no longer does; the zip URL this function used to return is a 404 today.
pub fn server_asset_url(release_tag: &str) -> String {
    format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{release_tag}/llama-{release_tag}-bin-macos-arm64.tar.gz"
    )
}

/// Unpacks the llama.cpp release tarball into `dest`.
///
/// **Flattened deliberately.** The archive nests everything under a
/// `llama-<tag>/` directory, and `llama-server` links its dozen `.dylib`s
/// through `@rpath` with `LC_RPATH = @loader_path` — meaning it looks for them
/// beside itself. Extracting the binary alone, or keeping the version prefix
/// while pointing `server_path()` at the root, both produce a binary that dies
/// at launch with a dyld error rather than anything about a missing download.
///
/// Entry paths are checked before use: a `..` component in an archive is the
/// classic path-traversal escape, and this one is fetched over the network.
pub fn extract_server(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive)
        .map_err(|e| format!("could not open the downloaded archive: {e}"))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));

    std::fs::create_dir_all(dest)
        .map_err(|e| format!("could not create {}: {e}", dest.display()))?;

    let entries = tar
        .entries()
        .map_err(|e| format!("the archive could not be read: {e}"))?;

    let mut found_server = false;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("the archive is damaged: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("the archive has an unreadable entry name: {e}"))?
            .into_owned();

        // Flatten: keep the file name, drop the `llama-<tag>/` prefix.
        let Some(name) = path.file_name() else {
            continue;
        };
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!(
                "the archive contains an unsafe path: {}",
                path.display()
            ));
        }
        let target = dest.join(name);
        let kind = entry.header().entry_type();

        if kind.is_symlink() {
            // **Not optional.** The release ships 18 symlinks, and the names
            // `llama-server` actually links against are among them:
            // `libllama-common.0.dylib` is a link to
            // `libllama-common.0.0.10229.dylib`. Skipping them extracts every
            // real file and still produces a binary that dies at launch with
            // `Library not loaded: @rpath/libllama-common.0.dylib`.
            let link = entry
                .link_name()
                .map_err(|e| format!("unreadable link target: {e}"))?
                .ok_or_else(|| format!("{} is a symlink with no target", name.to_string_lossy()))?
                .into_owned();

            // After flattening, every legitimate target is a bare filename in
            // the same directory. Anything else — absolute, or containing a
            // separator — is a link pointing out of the runtime directory.
            let link_name = link
                .file_name()
                .ok_or_else(|| format!("unsafe link target: {}", link.display()))?;
            if link.as_os_str() != link_name {
                return Err(format!("unsafe link target: {}", link.display()));
            }

            let _ = std::fs::remove_file(&target);
            std::os::unix::fs::symlink(link_name, &target)
                .map_err(|e| format!("could not link {}: {e}", name.to_string_lossy()))?;
            continue;
        }

        if !kind.is_file() {
            continue;
        }

        entry
            .unpack(&target)
            .map_err(|e| format!("could not extract {}: {e}", name.to_string_lossy()))?;

        if name == SERVER_BINARY {
            found_server = true;
            make_executable(&target)?;
        }
    }

    if !found_server {
        return Err(format!(
            "the archive did not contain {SERVER_BINARY} — the release layout may have changed"
        ));
    }
    Ok(())
}

/// The tar crate preserves modes, but a release that ships the binary
/// non-executable would fail at spawn with a bare permission error.
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| format!("could not stat {}: {e}", path.display()))?
        .permissions();
    perms.set_mode(perms.mode() | 0o755);
    std::fs::set_permissions(path, perms)
        .map_err(|e| format!("could not make {} executable: {e}", path.display()))
}

/// How much of a model is on disk.
///
/// `Partial` exists so the UI can offer "resume" rather than "download" and
/// show how far a previous attempt got — on a multi-gigabyte file, silently
/// restarting is the difference between a minute and an hour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ModelStatus {
    Absent,
    Partial { bytes: u64 },
    Installed,
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

    /// Downloads and installs `llama-server`, replacing any previous install.
    ///
    /// The archive is fetched to the runtime directory and deleted once
    /// unpacked. It is not kept: at 11 MB it is cheap to refetch, and a stale
    /// tarball beside the binary invites someone to wonder which one is live.
    pub async fn install_server(
        &self,
        http: &reqwest::Client,
        mut on_progress: impl FnMut(download::DownloadProgress),
        should_cancel: impl Fn() -> bool,
    ) -> Result<(), String> {
        std::fs::create_dir_all(&self.root)
            .map_err(|e| format!("could not create the runtime directory: {e}"))?;

        let archive = self.root.join("llama-server.tar.gz");
        download::download(
            http,
            &server_asset_url(SERVER_RELEASE),
            &archive,
            "server",
            &mut on_progress,
            should_cancel,
        )
        .await
        .map_err(|e| e.to_string())?;

        let result = extract_server(&archive, &self.root);
        // Removed either way: a failed extraction leaves a tarball that the
        // next attempt would otherwise resume as if it were partial.
        let _ = std::fs::remove_file(&archive);
        result?;

        on_progress(download::DownloadProgress {
            id: "server".into(),
            downloaded: 1,
            total: Some(1),
            done: true,
        });
        Ok(())
    }

    /// Downloads a model from the curated list.
    ///
    /// Verified before it counts as installed: a file that is the right size but
    /// is not a GGUF gets deleted rather than left to fail at generation time,
    /// which is hours later and looks like a different bug entirely.
    pub async fn install_model(
        &self,
        http: &reqwest::Client,
        model_id: &str,
        on_progress: impl FnMut(download::DownloadProgress),
        should_cancel: impl Fn() -> bool,
    ) -> Result<(), String> {
        let option = model_options()
            .into_iter()
            .find(|o| o.id == model_id)
            .ok_or_else(|| format!("no such model: {model_id}"))?;

        let dest = self.model_path(&option.filename);
        download::download(
            http,
            &option.url,
            &dest,
            &option.id,
            on_progress,
            should_cancel,
        )
        .await
        .map_err(|e| e.to_string())?;

        if !download::looks_like_gguf(&dest) {
            let _ = std::fs::remove_file(&dest);
            return Err(format!(
                "{} downloaded but is not a GGUF model — the URL may have moved",
                option.name
            ));
        }
        Ok(())
    }

    /// What is already on disk for a model: nothing, partial, or installed.
    pub fn model_status(&self, model_id: &str) -> ModelStatus {
        let Some(option) = model_options().into_iter().find(|o| o.id == model_id) else {
            return ModelStatus::Absent;
        };
        let dest = self.model_path(&option.filename);
        if self.installed_models().contains(&option.filename) {
            return ModelStatus::Installed;
        }
        match download::resumable_bytes(&dest) {
            0 => ModelStatus::Absent,
            bytes => ModelStatus::Partial { bytes },
        }
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
    fn the_server_asset_is_a_tarball_not_a_zip() {
        // llama.cpp stopped publishing macOS zips; the old URL 404s. Extraction
        // would fail on the wrong format anyway, but much later and less clearly.
        let url = server_asset_url(SERVER_RELEASE);
        assert!(url.ends_with(".tar.gz"), "wrong archive format: {url}");
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

    // MARK: extraction

    /// Builds a gzipped tar with the same shape as a llama.cpp release.
    fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        tarball_with_links(entries, &[])
    }

    /// As above, plus symlink entries — which the real release is full of.
    fn tarball_with_links(entries: &[(&str, &[u8])], links: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, body) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *body).unwrap();
        }
        for (name, target) in links {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o777);
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name(target).unwrap();
            header.set_cksum();
            builder
                .append_data(&mut header, name, std::io::empty())
                .unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();

        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn write_tarball(dir: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("server.tar.gz");
        std::fs::write(&path, tarball(entries)).unwrap();
        path
    }

    #[test]
    fn extraction_flattens_the_version_prefix() {
        // `llama-server` finds its dylibs through `@loader_path`, so they have
        // to end up beside it. Keeping `llama-b10229/` would give a binary that
        // dies at launch with a dyld error mentioning nothing about downloads.
        let dir = tempfile::tempdir().unwrap();
        let archive = write_tarball(
            dir.path(),
            &[
                ("llama-b10229/llama-server", b"#!/bin/sh\n"),
                ("llama-b10229/libllama.0.dylib", b"fake dylib"),
                ("llama-b10229/LICENSE", b"MIT"),
            ],
        );

        let out = dir.path().join("runtime");
        extract_server(&archive, &out).unwrap();

        assert!(out.join("llama-server").is_file());
        assert!(
            out.join("libllama.0.dylib").is_file(),
            "the dylib must sit beside the binary, not in a subdirectory"
        );
        assert!(!out.join("llama-b10229").exists());
    }

    #[test]
    fn the_extracted_server_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let archive = write_tarball(dir.path(), &[("llama-b1/llama-server", b"#!/bin/sh\n")]);

        let out = dir.path().join("runtime");
        extract_server(&archive, &out).unwrap();

        let mode = std::fs::metadata(out.join("llama-server"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "not executable: {mode:o}");
    }

    #[test]
    fn extraction_recreates_the_symlinked_library_names() {
        // The release links `libllama-common.0.dylib` to a versioned file, and
        // that link name is what the binary asks dyld for. Dropping symlinks
        // extracts every real file and still yields a binary that will not run.
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("s.tar.gz");
        std::fs::write(
            &archive,
            tarball_with_links(
                &[
                    ("llama-b1/llama-server", b"#!/bin/sh\n"),
                    ("llama-b1/libllama-common.0.0.1.dylib", b"real library"),
                ],
                &[(
                    "llama-b1/libllama-common.0.dylib",
                    "libllama-common.0.0.1.dylib",
                )],
            ),
        )
        .unwrap();

        let out = dir.path().join("runtime");
        extract_server(&archive, &out).unwrap();

        let link = out.join("libllama-common.0.dylib");
        assert!(
            link.symlink_metadata().is_ok(),
            "the symlink was dropped — the binary would fail to launch"
        );
        assert_eq!(
            std::fs::read(&link).unwrap(),
            b"real library",
            "the link does not resolve to its target"
        );
    }

    #[test]
    fn a_symlink_pointing_outside_the_runtime_is_refused() {
        // A link is a write primitive too: left unchecked, a later extraction
        // could follow it out of the directory.
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.tar.gz");
        std::fs::write(
            &archive,
            tarball_with_links(
                &[("llama-b1/llama-server", b"#!/bin/sh\n")],
                &[("llama-b1/sneaky.dylib", "../../../../etc/passwd")],
            ),
        )
        .unwrap();

        let err = extract_server(&archive, &dir.path().join("runtime")).unwrap_err();
        assert!(err.contains("unsafe link target"), "unhelpful error: {err}");
    }

    #[test]
    fn an_absolute_symlink_target_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil2.tar.gz");
        std::fs::write(
            &archive,
            tarball_with_links(
                &[("llama-b1/llama-server", b"#!/bin/sh\n")],
                &[("llama-b1/sneaky.dylib", "/etc/passwd")],
            ),
        )
        .unwrap();

        let err = extract_server(&archive, &dir.path().join("runtime")).unwrap_err();
        assert!(err.contains("unsafe link target"), "unhelpful error: {err}");
    }

    #[test]
    fn an_archive_without_the_server_is_rejected_with_a_reason() {
        // The release layout changing upstream should say so, not leave a
        // directory of dylibs and a runtime that claims to be uninstalled.
        let dir = tempfile::tempdir().unwrap();
        let archive = write_tarball(dir.path(), &[("llama-b1/libllama.dylib", b"x")]);

        let err = extract_server(&archive, &dir.path().join("runtime")).unwrap_err();
        assert!(err.contains(SERVER_BINARY), "unhelpful error: {err}");
        assert!(err.contains("layout"), "unhelpful error: {err}");
    }

    /// Hand-builds a tar containing a path the `tar` crate's builder refuses to
    /// write — which is the point: the guard has to hold against an archive
    /// this process did not create.
    fn malicious_tarball(name: &str, body: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(b"0000644");
        header[108..115].copy_from_slice(b"0000000");
        header[116..123].copy_from_slice(b"0000000");
        let size = format!("{:011o}", body.len());
        header[124..135].copy_from_slice(size.as_bytes());
        header[136..147].copy_from_slice(b"00000000000");
        header[148..156].copy_from_slice(b"        "); // checksum placeholder
        header[156] = b'0'; // regular file

        let checksum: u32 = header.iter().map(|b| *b as u32).sum();
        let rendered = format!("{checksum:06o}\0 ");
        header[148..156].copy_from_slice(rendered.as_bytes());

        let mut tar_bytes = header.to_vec();
        tar_bytes.extend_from_slice(body);
        tar_bytes.resize(tar_bytes.len().div_ceil(512) * 512, 0);
        tar_bytes.extend_from_slice(&[0u8; 1024]); // end-of-archive

        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn an_archive_that_tries_to_escape_its_directory_is_refused() {
        // This tarball arrives over the network. A `..` entry writing outside
        // the runtime directory is the classic tar traversal, and the `tar`
        // crate's *builder* will not even produce one — so it is forged here.
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.tar.gz");
        std::fs::write(&archive, malicious_tarball("../../escaped.txt", b"pwned")).unwrap();

        let err = extract_server(&archive, &dir.path().join("runtime")).unwrap_err();
        assert!(err.contains("unsafe path"), "unhelpful error: {err}");
        assert!(
            !dir.path().join("escaped.txt").exists(),
            "the archive wrote outside its destination"
        );
        assert!(!dir.path().parent().unwrap().join("escaped.txt").exists());
    }

    #[test]
    fn a_corrupt_archive_reports_itself_rather_than_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("broken.tar.gz");
        std::fs::write(&archive, b"this is not a gzip stream at all").unwrap();

        let err = extract_server(&archive, &dir.path().join("runtime")).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn extracting_over_a_previous_install_replaces_it() {
        // Upgrading the pinned release must not leave last version's binary.
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("runtime");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("llama-server"), b"old build").unwrap();

        let archive = write_tarball(dir.path(), &[("llama-b2/llama-server", b"new build")]);
        extract_server(&archive, &out).unwrap();

        assert_eq!(
            std::fs::read(out.join("llama-server")).unwrap(),
            b"new build"
        );
    }

    #[test]
    fn a_model_with_nothing_on_disk_is_absent() {
        let (_dir, runtime) = temp_runtime();
        assert_eq!(runtime.model_status("qwen2.5-3b"), ModelStatus::Absent);
    }

    #[test]
    fn a_half_downloaded_model_reports_how_far_it_got() {
        // So the UI can say "resume from 1.2 GB" instead of starting again.
        let (_dir, runtime) = temp_runtime();
        let option = model_options()
            .into_iter()
            .find(|o| o.id == "qwen2.5-3b")
            .unwrap();
        std::fs::create_dir_all(runtime.models_dir()).unwrap();
        std::fs::write(
            crate::llm::download::part_path(&runtime.model_path(&option.filename)),
            vec![0u8; 4096],
        )
        .unwrap();

        assert_eq!(
            runtime.model_status("qwen2.5-3b"),
            ModelStatus::Partial { bytes: 4096 }
        );
    }

    #[test]
    fn a_finished_model_reports_installed() {
        let (_dir, runtime) = temp_runtime();
        let option = model_options()
            .into_iter()
            .find(|o| o.id == "qwen2.5-3b")
            .unwrap();
        std::fs::create_dir_all(runtime.models_dir()).unwrap();
        std::fs::write(runtime.model_path(&option.filename), vec![0u8; 150_000_000]).unwrap();

        assert_eq!(runtime.model_status("qwen2.5-3b"), ModelStatus::Installed);
    }

    #[test]
    fn an_unknown_model_id_is_absent_rather_than_a_panic() {
        let (_dir, runtime) = temp_runtime();
        assert_eq!(runtime.model_status("not-a-model"), ModelStatus::Absent);
    }

    #[tokio::test]
    async fn installing_an_unknown_model_says_so() {
        let (_dir, runtime) = temp_runtime();
        let err = runtime
            .install_model(&reqwest::Client::new(), "nope", |_| {}, || false)
            .await
            .unwrap_err();
        assert!(err.contains("no such model"), "unhelpful error: {err}");
    }

    // MARK: live network checks
    //
    // Ignored by default: CI has no business fetching from GitHub and Hugging
    // Face on every push, and the server download is 11 MB. Run them when
    // bumping `SERVER_RELEASE` or touching the model list — they are the only
    // things that catch an upstream URL moving, which is exactly how both of
    // these were broken before.
    //
    //   cargo test --lib live_ -- --ignored --nocapture

    #[tokio::test]
    #[ignore]
    async fn live_the_pinned_server_release_downloads_extracts_and_runs() {
        let (_dir, runtime) = temp_runtime();
        let started = std::time::Instant::now();

        runtime
            .install_server(&reqwest::Client::new(), |_| {}, || false)
            .await
            .expect("install failed");

        eprintln!(
            "installed {} in {:.1}s",
            SERVER_RELEASE,
            started.elapsed().as_secs_f32()
        );

        let server = runtime.server_path();
        assert!(server.is_file(), "no binary at {}", server.display());

        // The real check: it links against a dozen dylibs through
        // `@loader_path`, so a flattening mistake shows up only when it runs.
        let output = Command::new(&server)
            .arg("--version")
            .output()
            .expect("could not execute the extracted server");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("llama-server --version: {}", text.trim());

        // Checked explicitly, because the loose version of this assertion
        // passed while the binary was failing to launch at all: a dyld error
        // mentions the missing library and happens to contain neither word
        // this used to look for.
        assert!(
            !text.contains("dyld") && !text.contains("Library not loaded"),
            "the binary did not launch — dynamic libraries are missing:\n{text}"
        );
        assert!(
            text.contains("version") || text.contains("build"),
            "the binary ran but said something unexpected: {text}"
        );
        assert_eq!(runtime.state(), RuntimeState::NeedsModel);
    }

    #[tokio::test]
    #[ignore]
    async fn live_every_model_url_serves_a_real_gguf() {
        // Catches a moved or renamed file without pulling gigabytes: the first
        // four bytes are enough to tell a model from an HTML error page, and
        // `Content-Range` confirms the full size.
        let http = reqwest::Client::new();
        for option in model_options() {
            let response = http
                .get(&option.url)
                .header(reqwest::header::RANGE, "bytes=0-3")
                .send()
                .await
                .unwrap_or_else(|e| panic!("{} unreachable: {e}", option.id));

            assert!(
                response.status().is_success(),
                "{} returned {} — the URL has moved",
                option.id,
                response.status()
            );

            let total = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next().map(str::to_string));

            let body = response.bytes().await.expect("no body");
            assert_eq!(
                &body[..4],
                download::GGUF_MAGIC,
                "{} is not a GGUF file",
                option.id
            );

            eprintln!("{}: ok, {} bytes", option.id, total.unwrap_or_default());
        }
    }

    /// G13's done-when, end to end: no API key, no Ollama, nothing but what
    /// this code downloads. Uses a 0.5B model rather than a curated 2–5 GB one
    /// — the curated URLs are checked separately by
    /// `live_every_model_url_serves_a_real_gguf`, and what this proves is the
    /// *path*: fetch, extract, launch, generate.
    ///
    ///   cargo test --lib live_a_bare_machine -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_a_bare_machine_can_generate_after_downloading() {
        const SMALL_MODEL: &str =
            "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf";

        let (_dir, runtime) = temp_runtime();
        let http = reqwest::Client::new();

        let started = std::time::Instant::now();
        runtime
            .install_server(&http, |_| {}, || false)
            .await
            .expect("server install failed");
        eprintln!(
            "server installed in {:.1}s",
            started.elapsed().as_secs_f32()
        );

        let started = std::time::Instant::now();
        let mut last = 0u64;
        download::download(
            &http,
            SMALL_MODEL,
            &runtime.model_path("qwen2.5-0.5b-instruct-q4_k_m.gguf"),
            "small",
            |p| last = p.downloaded,
            || false,
        )
        .await
        .expect("model download failed");
        eprintln!(
            "downloaded {last} bytes in {:.1}s",
            started.elapsed().as_secs_f32()
        );

        assert_eq!(runtime.state(), RuntimeState::Ready, "runtime not ready");

        let pid = runtime.start().expect("server would not start");
        eprintln!("llama-server running as {pid}");

        // Wait for it to load the model and start listening.
        let mut ready = false;
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if http
                .get(format!("http://127.0.0.1:{PORT}/health"))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                ready = true;
                break;
            }
        }
        assert!(ready, "the server never became healthy");

        let response = http
            .post(format!("http://127.0.0.1:{PORT}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "local",
                "messages": [{
                    "role": "user",
                    "content": "Reply with exactly the word: oatmeal"
                }],
                "max_tokens": 20,
            }))
            .send()
            .await
            .expect("no response from the local server");

        assert!(response.status().is_success(), "got {}", response.status());
        let body: serde_json::Value = response.json().await.expect("bad json");
        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .expect("no completion in the response")
            .to_string();

        eprintln!("local model replied: {text:?}");
        assert!(!text.trim().is_empty(), "the model returned nothing");

        runtime.stop();
    }

    #[test]
    fn stopping_a_runtime_that_never_started_is_harmless() {
        let (_dir, runtime) = temp_runtime();
        runtime.stop();
        assert_eq!(runtime.state(), RuntimeState::NotInstalled);
    }
}
