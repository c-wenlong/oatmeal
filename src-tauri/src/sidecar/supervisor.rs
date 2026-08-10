//! Spawns the Swift sidecar, reads its event stream, and restarts it when it dies.
//!
//! Built on `std::process` rather than Tauri's shell plugin so the whole thing
//! is exercisable from an ordinary integration test against the real binary.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use super::policy::{RestartPolicy, RestartTracker};
use super::protocol::{check_ready, encode_command, parse_event, SidecarCommand, SidecarEvent};

/// Everything the supervisor reports upward. Lifecycle and protocol events share
/// one stream so the UI can show them interleaved in the order they happened.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SupervisorEvent {
    Spawned {
        pid: u32,
        attempt: u32,
    },
    Event {
        event: SidecarEvent,
    },
    /// A line that didn't parse. Non-fatal — we log and keep reading, since one
    /// bad line shouldn't end a recording.
    Garbled {
        line: String,
        error: String,
    },
    /// A diagnostic line the sidecar wrote to stderr.
    ///
    /// Reported rather than printed. In a bundled `.app` the Rust process has
    /// no terminal, so `eprintln!` sent the only window into what capture, the
    /// ASR model and the calendar watcher were doing straight to nowhere.
    Stderr {
        line: String,
    },
    Exited {
        code: Option<i32>,
        restarting_in_ms: Option<u64>,
    },
    GaveUp {
        reason: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error("sidecar binary not found; looked in: {searched}")]
    BinaryNotFound { searched: String },
    #[error("sidecar is not running")]
    NotRunning,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Name Tauri bundles the sidecar under (the target triple is stripped on bundle).
const SIDECAR_NAME: &str = "oatmeal-sidecar";

/// The places a sidecar binary might live, most specific first.
///
/// Split from [`first_existing`] so the selection rule can be tested against
/// paths the test controls. Testing `resolve_binary` directly would mean
/// mutating the process-global `OATMEAL_SIDECAR_PATH`, which races other tests
/// in the same binary and is unsound in a multithreaded program.
fn candidate_paths() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Explicit override wins — used by anyone debugging a locally built sidecar.
    if let Ok(path) = std::env::var("OATMEAL_SIDECAR_PATH") {
        candidates.push(PathBuf::from(path));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Bundled: Tauri copies external binaries next to the main binary.
            candidates.push(dir.join(SIDECAR_NAME));
            candidates.push(dir.join(format!("{SIDECAR_NAME}-{}", target_triple())));
        }
    }

    // `tauri dev` runs from target/debug and does not stage externalBin, so fall
    // back to where `pnpm sidecar:build` puts it.
    // Fully qualified rather than imported: `Path` is used only here, so a
    // top-level import is unused in a release build and warns on every one.
    #[cfg(debug_assertions)]
    candidates.push(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(format!("{SIDECAR_NAME}-{}", target_triple())),
    );

    candidates
}

/// Returns the first candidate that is a real file, or an error naming every
/// path tried — so a missing sidecar is actionable rather than just "not found".
fn first_existing(candidates: Vec<PathBuf>) -> Result<PathBuf, SidecarError> {
    let mut searched = Vec::new();

    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
        searched.push(candidate.display().to_string());
    }

    Err(SidecarError::BinaryNotFound {
        searched: searched.join(", "),
    })
}

/// Finds the sidecar binary across dev and bundled layouts.
pub fn resolve_binary() -> Result<PathBuf, SidecarError> {
    first_existing(candidate_paths())
}

fn target_triple() -> &'static str {
    // Only ever built for Apple Silicon (SPEC section 2).
    "aarch64-apple-darwin"
}

struct Shared {
    binary: PathBuf,
    args: Vec<String>,
    running: AtomicBool,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
}

pub struct Supervisor {
    shared: Arc<Shared>,
}

impl Supervisor {
    pub fn new(binary: PathBuf, args: Vec<String>) -> Self {
        Self {
            shared: Arc::new(Shared {
                binary,
                args,
                running: AtomicBool::new(false),
                stdin: Mutex::new(None),
                child: Mutex::new(None),
            }),
        }
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }

    /// Starts the supervise loop on a background thread. `on_event` is called
    /// for every lifecycle and protocol event, in order.
    pub fn start<F>(&self, policy: RestartPolicy, on_event: F)
    where
        F: Fn(SupervisorEvent) + Send + Sync + 'static,
    {
        if self.shared.running.swap(true, Ordering::SeqCst) {
            return; // already supervising
        }

        let shared = Arc::clone(&self.shared);
        let on_event = Arc::new(on_event);

        std::thread::spawn(move || {
            let mut tracker = RestartTracker::new(policy);
            let mut attempt = 0u32;

            while shared.running.load(Ordering::SeqCst) {
                attempt += 1;
                let started = Instant::now();

                let mut child = match Command::new(&shared.binary)
                    .args(&shared.args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                {
                    Ok(child) => child,
                    Err(err) => {
                        // Flag first, then announce: an observer that reacts to
                        // GaveUp must never see `is_running()` still true.
                        shared.running.store(false, Ordering::SeqCst);
                        on_event(SupervisorEvent::GaveUp {
                            reason: format!("could not spawn sidecar: {err}"),
                        });
                        return;
                    }
                };

                on_event(SupervisorEvent::Spawned {
                    pid: child.id(),
                    attempt,
                });

                let stdout = child.stdout.take().expect("stdout was piped");

                // The sidecar's diagnostics are the only window into what audio
                // capture and the ASR model are doing. Discarding them, as this
                // used to, makes every failure in there invisible.
                if let Some(stderr) = child.stderr.take() {
                    let sink = on_event.clone();
                    std::thread::spawn(move || {
                        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                            sink(SupervisorEvent::Stderr { line });
                        }
                    });
                }
                *shared.stdin.lock().unwrap() = child.stdin.take();

                // Reading happens on this thread; the child handle goes into the
                // mutex so `stop()` and the UI's kill button can reach it.
                *shared.child.lock().unwrap() = Some(child);

                for line in BufReader::new(stdout).lines() {
                    let line = match line {
                        Ok(line) => line,
                        Err(_) => break, // pipe closed
                    };

                    match parse_event(&line) {
                        Ok(None) => continue,
                        Ok(Some(event)) => {
                            // Note: reaching Ready deliberately does NOT reset
                            // the restart budget. A sidecar that handshakes and
                            // then dies on the next command would otherwise
                            // restart forever; only uptime counts as healthy.
                            if let SidecarEvent::Ready { .. } = event {
                                if let Err(err) = check_ready(&event) {
                                    shared.running.store(false, Ordering::SeqCst);
                                    on_event(SupervisorEvent::GaveUp {
                                        reason: err.to_string(),
                                    });
                                    break;
                                }
                            }
                            on_event(SupervisorEvent::Event { event });
                        }
                        Err(err) => on_event(SupervisorEvent::Garbled {
                            line,
                            error: err.to_string(),
                        }),
                    }
                }

                // Stdout closed: reap and decide whether to go again.
                let status = shared
                    .child
                    .lock()
                    .unwrap()
                    .as_mut()
                    .and_then(|c| c.wait().ok());
                *shared.stdin.lock().unwrap() = None;
                *shared.child.lock().unwrap() = None;

                let code = status.and_then(|s| s.code());

                if !shared.running.load(Ordering::SeqCst) {
                    on_event(SupervisorEvent::Exited {
                        code,
                        restarting_in_ms: None,
                    });
                    return;
                }

                match tracker.record_exit(started.elapsed()) {
                    Some(delay) => {
                        on_event(SupervisorEvent::Exited {
                            code,
                            restarting_in_ms: Some(delay.as_millis() as u64),
                        });
                        std::thread::sleep(delay);
                    }
                    None => {
                        on_event(SupervisorEvent::Exited {
                            code,
                            restarting_in_ms: None,
                        });
                        shared.running.store(false, Ordering::SeqCst);
                        on_event(SupervisorEvent::GaveUp {
                            reason: format!(
                                "sidecar exited {} times in a row",
                                tracker.consecutive_failures()
                            ),
                        });
                        return;
                    }
                }
            }
        });
    }

    /// Writes a command to the sidecar's stdin.
    pub fn send(&self, command: &SidecarCommand) -> Result<(), SidecarError> {
        let mut guard = self.shared.stdin.lock().unwrap();
        let stdin = guard.as_mut().ok_or(SidecarError::NotRunning)?;
        stdin.write_all(encode_command(command).as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    /// Kills the child without clearing the running flag, so the supervise loop
    /// treats it as a crash and restarts. This is what the UI's "simulate crash"
    /// button uses.
    pub fn kill_child(&self) -> Result<(), SidecarError> {
        let mut guard = self.shared.child.lock().unwrap();
        let child = guard.as_mut().ok_or(SidecarError::NotRunning)?;
        child.kill()?;
        Ok(())
    }

    /// Stops supervising and terminates the child. Idempotent.
    pub fn stop(&self) {
        self.shared.running.store(false, Ordering::SeqCst);
        // Dropping stdin gives the sidecar its normal EOF shutdown path; the
        // kill is the backstop for one that ignores it.
        *self.shared.stdin.lock().unwrap() = None;
        if let Some(child) = self.shared.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn missing_binary_names_every_path_it_tried() {
        let err = first_existing(vec![
            PathBuf::from("/nonexistent/first"),
            PathBuf::from("/nonexistent/second"),
        ])
        .expect_err("nothing exists, so this must fail");

        let SidecarError::BinaryNotFound { searched } = err else {
            panic!("wrong error variant: {err:?}");
        };
        // Naming only the last path tried would send someone debugging to the
        // wrong place, so assert both appear.
        assert!(searched.contains("/nonexistent/first"), "got: {searched}");
        assert!(searched.contains("/nonexistent/second"), "got: {searched}");
    }

    #[test]
    fn resolution_prefers_the_earliest_existing_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let preferred = dir.path().join("preferred");
        let fallback = dir.path().join("fallback");
        std::fs::write(&preferred, b"#!/bin/sh\n").unwrap();
        std::fs::write(&fallback, b"#!/bin/sh\n").unwrap();

        // Order is the whole contract: an explicit override must beat the
        // bundled copy, which must beat the dev fallback.
        let found = first_existing(vec![
            PathBuf::from("/nonexistent/missing"),
            preferred.clone(),
            fallback,
        ])
        .expect("one candidate exists");

        assert_eq!(found, preferred);
    }

    #[test]
    fn a_directory_is_not_mistaken_for_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let decoy = dir.path().join("oatmeal-sidecar");
        std::fs::create_dir(&decoy).unwrap();
        let real = dir.path().join("real");
        std::fs::write(&real, b"#!/bin/sh\n").unwrap();

        // `exists()` would accept the directory and we'd fail later at spawn
        // with a confusing "permission denied"; `is_file()` skips it here.
        let found = first_existing(vec![decoy, real.clone()]).expect("should skip the dir");
        assert_eq!(found, real);
    }

    #[test]
    fn the_real_candidate_list_is_never_empty() {
        // A resolution order that silently produced no candidates would report
        // "looked in: " with nothing after it.
        assert!(!candidate_paths().is_empty());
    }

    #[test]
    fn spawn_failure_gives_up_rather_than_looping() {
        let supervisor = Supervisor::new(PathBuf::from("/nonexistent/oatmeal-sidecar"), vec![]);
        let (tx, rx) = std::sync::mpsc::channel();
        supervisor.start(RestartPolicy::default(), move |event| {
            let _ = tx.send(event);
        });

        let event = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("supervisor should report the failure");
        assert!(matches!(event, SupervisorEvent::GaveUp { .. }));
        assert!(!supervisor.is_running());
    }
}
