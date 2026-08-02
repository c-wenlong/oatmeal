pub mod db;
pub mod meeting;
pub mod sidecar;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};

use db::{repo, Database};
use meeting::{MeetingEvent, MeetingState};
use sidecar::{Supervisor, SupervisorEvent};

/// Reported to the frontend by [`health_check`]. Mirrored by `HealthInfo` in
/// `src/types.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthInfo {
    pub app_version: String,
    pub build_profile: String,
    pub arch: String,
    pub os: String,
}

impl HealthInfo {
    pub fn current() -> Self {
        Self {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            build_profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
            .to_string(),
            arch: std::env::consts::ARCH.to_string(),
            os: std::env::consts::OS.to_string(),
        }
    }
}

#[tauri::command]
fn health_check() -> HealthInfo {
    HealthInfo::current()
}

/// Long-lived app state. The database is behind a `Mutex` rather than a pool
/// because SQLite in WAL mode wants one writer and the read volume here is tiny.
pub struct AppState {
    pub db: Mutex<Database>,
    pub db_path: String,
    /// `None` until `sidecar_start` is called, so a missing sidecar binary
    /// degrades to "that card doesn't work" rather than the app refusing to boot.
    ///
    /// `Arc` so the supervisor can be handed to the event callback without the
    /// callback needing this mutex — it fires on the supervisor thread and would
    /// otherwise deadlock against the caller that is still holding the lock.
    pub sidecar: Mutex<Option<Arc<Supervisor>>>,
    /// Where the meeting lifecycle currently is. Every transition goes through
    /// `meeting::next`, so illegal ones are refused rather than silently
    /// producing a half-state.
    pub meeting: Mutex<MeetingState>,
    /// Last permission snapshot the sidecar reported.
    ///
    /// Cached because permissions arrive as a one-shot event: anything that
    /// subscribes afterwards (a later mount, a hot reload, a second window)
    /// would otherwise show "unknown" forever despite the answer being known.
    pub last_permissions: Mutex<Option<PermissionsSnapshot>>,
}

/// Mirrors `PermissionsSnapshot` in `src/lib/permissions.ts`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsSnapshot {
    pub microphone: sidecar::PermissionState,
    pub screen_recording: sidecar::PermissionState,
    pub needs_relaunch: bool,
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Persists a settled transcript line against the meeting being recorded.
///
/// Only `Final` events land here. `Partial`s are in-flight text that a later
/// final supersedes — writing them would duplicate every sentence.
fn persist_final(app: &tauri::AppHandle, event: &sidecar::SidecarEvent) {
    let sidecar::SidecarEvent::Final {
        source,
        text,
        t0,
        t1,
        conf,
    } = event
    else {
        return;
    };

    let state = app.state::<AppState>();
    let Ok(machine) = state.meeting.lock() else {
        return;
    };
    // `Processing` still counts: audio that settled just before the stop
    // belongs to that meeting rather than nowhere.
    let Some(meeting_id) = machine.active_meeting().map(str::to_string) else {
        return;
    };
    drop(machine);

    let Ok(db) = state.db.lock() else { return };
    if let Err(err) = repo::append_utterance(
        db.connection(),
        &meeting_id,
        source.as_str(),
        text,
        *t0,
        *t1,
        *conf,
    ) {
        eprintln!("failed to persist utterance: {err}");
    }
}

/// Closes out the meeting row once the sidecar confirms the file is final.
fn finish_active_meeting(app: &tauri::AppHandle, event: &sidecar::SidecarEvent) {
    let sidecar::SidecarEvent::Stopped { audio_path, .. } = event else {
        return;
    };

    let state = app.state::<AppState>();
    let Ok(machine) = state.meeting.lock() else {
        return;
    };
    let Some(meeting_id) = machine.active_meeting().map(str::to_string) else {
        return;
    };
    drop(machine);
    // The sidecar confirming a stop is what actually ends a meeting.
    let _ = transition(app, MeetingEvent::SidecarStopped);

    let Ok(db) = state.db.lock() else { return };
    // Default retention is 7 days (SPEC section 11); the sweeper in G27 deletes
    // the file and leaves the transcript.
    let expires = now_ms() + 7 * 24 * 60 * 60 * 1000;
    if let Err(err) = repo::finish_meeting(
        db.connection(),
        &meeting_id,
        now_ms(),
        audio_path.as_deref(),
        Some(expires),
    ) {
        eprintln!("failed to finish meeting: {err}");
    }
}

/// Creates a meeting row and tells the sidecar to start recording into it.
#[tauri::command]
fn meeting_start(app: tauri::AppHandle, title: Option<String>) -> Result<String, String> {
    let state = app.state::<AppState>();

    let started = now_ms();
    let id = format!("m{started}");
    let title = title.unwrap_or_else(|| "Untitled meeting".to_string());

    {
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        repo::insert_meeting(db.connection(), &id, &title, started).map_err(|e| e.to_string())?;
    }

    // The row exists before the sidecar is told to record, so an utterance that
    // arrives immediately has somewhere to go.
    transition(
        &app,
        MeetingEvent::Started {
            meeting_id: id.clone(),
        },
    )?;

    let guard = state.sidecar.lock().map_err(|_| "sidecar lock poisoned")?;
    let supervisor = guard
        .as_ref()
        .ok_or_else(|| "sidecar is not running".to_string())?;
    supervisor
        .send(&sidecar::SidecarCommand::Start {
            meeting_id: id.clone(),
            sources: vec![sidecar::AudioSource::Mic, sidecar::AudioSource::System],
        })
        .map_err(|e| e.to_string())?;

    Ok(id)
}

#[tauri::command]
fn meeting_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    {
        let mut machine = state.meeting.lock().map_err(|_| "lock poisoned")?;
        *machine =
            meeting::next(&machine, MeetingEvent::StopRequested).map_err(|e| e.to_string())?;
    }

    let guard = state.sidecar.lock().map_err(|_| "sidecar lock poisoned")?;
    guard
        .as_ref()
        .ok_or_else(|| "sidecar is not running".to_string())?
        .send(&sidecar::SidecarCommand::Stop)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn meeting_active(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state
        .meeting
        .lock()
        .map_err(|_| "lock poisoned")?
        .active_meeting()
        .map(str::to_string))
}

/// Full lifecycle state, so the UI can distinguish "finalising" from "idle".
#[tauri::command]
fn meeting_state(state: tauri::State<'_, AppState>) -> Result<MeetingState, String> {
    Ok(state.meeting.lock().map_err(|_| "lock poisoned")?.clone())
}

#[tauri::command]
fn meeting_rename(
    meeting_id: String,
    title: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("a meeting needs a title".into());
    }
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::rename_meeting(db.connection(), &meeting_id, trimmed).map_err(|e| e.to_string())
}

/// Deletes a meeting, its transcript, its notes and its audio file.
#[tauri::command]
fn meeting_delete(meeting_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    {
        // Refusing here rather than deleting out from under the capture engine,
        // which would leave the sidecar writing to a meeting that no longer exists.
        let machine = state.meeting.lock().map_err(|_| "lock poisoned")?;
        if machine.active_meeting() == Some(meeting_id.as_str()) {
            return Err("stop the recording before deleting it".into());
        }
    }

    let audio_path = {
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        repo::delete_meeting(db.connection(), &meeting_id).map_err(|e| e.to_string())?
    };

    // Best effort: the rows are already gone, and a leftover file is better
    // than an error that makes the delete look like it failed.
    if let Some(path) = audio_path {
        if let Err(err) = std::fs::remove_file(&path) {
            eprintln!("could not remove {path}: {err}");
        }
    }
    Ok(())
}

/// Saves the whole notepad for a meeting.
///
/// The editor owns block identity and timing; Rust only enforces that a
/// block's `firstTypedAtMs` is never rewritten once set.
#[tauri::command]
fn notes_save(
    meeting_id: String,
    blocks: Vec<repo::NoteBlock>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::save_note_blocks(db.connection_mut(), &meeting_id, &blocks).map_err(|e| e.to_string())
}

#[tauri::command]
fn notes_load(
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<repo::NoteBlock>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::meeting_notes(db.connection(), &meeting_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn meetings_list(state: tauri::State<'_, AppState>) -> Result<Vec<repo::MeetingSummary>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::list_meetings(db.connection(), 50).map_err(|e| e.to_string())
}

#[tauri::command]
fn meeting_transcript(
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<repo::Utterance>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::meeting_utterances(db.connection(), &meeting_id).map_err(|e| e.to_string())
}

/// Returns the cached snapshot, or `None` if the sidecar has never reported.
#[tauri::command]
fn permissions_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<Option<PermissionsSnapshot>, String> {
    Ok(*state
        .last_permissions
        .lock()
        .map_err(|_| "permissions lock poisoned")?)
}

/// Tauri event channel carrying every [`SupervisorEvent`] to the frontend.
pub const SIDECAR_EVENT: &str = "sidecar://event";

/// Tauri event carrying every [`MeetingState`] change to the frontend.
pub const MEETING_STATE_EVENT: &str = "meeting://state";

/// Applies a lifecycle transition and tells the frontend.
///
/// Rust owns this state. A meeting can be started by something other than the
/// record button — the dev harness today, calendar detection later — and a
/// frontend that only reads the state on mount shows "idle" while a recording
/// is plainly running.
fn transition(app: &tauri::AppHandle, event: MeetingEvent) -> Result<MeetingState, String> {
    let state = app.state::<AppState>();
    let mut machine = state.meeting.lock().map_err(|_| "lock poisoned")?;
    let next_state = meeting::next(&machine, event).map_err(|e| e.to_string())?;
    *machine = next_state.clone();
    drop(machine);

    let _ = app.emit(MEETING_STATE_EVENT, &next_state);
    Ok(next_state)
}

/// Set to `1` to spawn the sidecar and run one scripted session at launch.
///
/// Dev-only affordance for the Phase 0 harness: it makes the end-to-end path
/// observable on a machine where synthetic clicks are blocked (macOS withholds
/// Accessibility permission from most automation). Unset, nothing happens.
const AUTOSTART_ENV: &str = "OATMEAL_HARNESS_AUTOSTART";

/// Seconds to record when the harness drives a full end-to-end capture.
/// Unset means "arm only" — nothing is written to disk.
fn harness_record_seconds() -> Option<u64> {
    std::env::var("OATMEAL_HARNESS_RECORD_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|seconds| *seconds > 0)
}

/// Spawns the sidecar and streams its events to the frontend.
///
/// Idempotent: calling it while a supervisor is already running is refused, so
/// double-clicking the button can't leave an orphaned process.
fn spawn_sidecar(app: &tauri::AppHandle, autorun_session: bool) -> Result<String, String> {
    let state = app.state::<AppState>();
    let mut guard = state.sidecar.lock().map_err(|_| "sidecar lock poisoned")?;

    if guard.as_ref().is_some_and(|s| s.is_running()) {
        return Err("sidecar is already running".into());
    }

    let binary = sidecar::resolve_binary().map_err(|e| e.to_string())?;
    let path = binary.display().to_string();

    // Real capture by default. `OATMEAL_SIDECAR_FIXTURE=1` swaps in the scripted
    // transcript, which is useful when demoing on a machine without permissions.
    let args = if std::env::var("OATMEAL_SIDECAR_FIXTURE").as_deref() == Ok("1") {
        vec!["--fixture".to_string(), "--fast".to_string()]
    } else {
        Vec::new()
    };
    let supervisor = Arc::new(Supervisor::new(binary, args));

    let app_handle = app.clone();
    let for_callback = Arc::clone(&supervisor);
    // One-shot: a sidecar restart re-emits `Model::Ready`, and without this the
    // harness would start a fresh meeting every time.
    let harness_fired = Arc::new(AtomicBool::new(false));
    supervisor.start(Default::default(), move |event| {
        // Cache before emitting, so a listener that reacts to the event and
        // immediately queries can't observe a staler snapshot than the event.
        if let SupervisorEvent::Event {
            event:
                sidecar::SidecarEvent::Permissions {
                    microphone,
                    screen_recording,
                    needs_relaunch,
                },
        } = &event
        {
            if let Ok(mut cached) = app_handle.state::<AppState>().last_permissions.lock() {
                *cached = Some(PermissionsSnapshot {
                    microphone: *microphone,
                    screen_recording: *screen_recording,
                    needs_relaunch: *needs_relaunch,
                });
            }
        }

        if let SupervisorEvent::Event { event: inner } = &event {
            persist_final(&app_handle, inner);
            finish_active_meeting(&app_handle, inner);
        }

        let _ = app_handle.emit(SIDECAR_EVENT, &event);

        if autorun_session
            && matches!(
                &event,
                SupervisorEvent::Event {
                    event: sidecar::SidecarEvent::Ready { .. }
                }
            )
        {
            // Query only — never `request: true` from an automated path, or the
            // harness would fire system dialogs at whoever launched it.
            let _ = for_callback.send(&sidecar::SidecarCommand::Permissions { request: false });
            // Arm so the pre-roll is filling by the time the model is ready.
            let _ = for_callback.send(&sidecar::SidecarCommand::Arm);
            let _ = transition(&app_handle, MeetingEvent::Armed);
        }

        // Harness: once the model is ready, run one timed recording end to end.
        // Same code path as the UI button — it goes through `meeting_start`, so
        // the meeting row and persistence are exercised, not bypassed.
        if let (true, Some(seconds)) = (autorun_session, harness_record_seconds()) {
            if !harness_fired.load(Ordering::SeqCst)
                && matches!(
                    &event,
                    SupervisorEvent::Event {
                        event: sidecar::SidecarEvent::Model {
                            state: sidecar::ModelState::Ready,
                            ..
                        }
                    }
                )
            {
                harness_fired.store(true, Ordering::SeqCst);
                let handle = app_handle.clone();
                std::thread::spawn(move || {
                    match meeting_start(handle.clone(), Some("Harness recording".into())) {
                        Ok(id) => eprintln!("harness: recording meeting {id}"),
                        Err(err) => {
                            eprintln!("harness: could not start meeting: {err}");
                            return;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(seconds));
                    let state = handle.state::<AppState>();
                    if let Err(err) = meeting_stop(state) {
                        eprintln!("harness: could not stop meeting: {err}");
                    }
                });
            }
        }
    });

    *guard = Some(supervisor);
    Ok(path)
}

#[tauri::command]
fn sidecar_start(app: tauri::AppHandle) -> Result<String, String> {
    spawn_sidecar(&app, false)
}

#[tauri::command]
fn sidecar_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.sidecar.lock().map_err(|_| "sidecar lock poisoned")?;
    if let Some(supervisor) = guard.take() {
        supervisor.stop();
    }
    Ok(())
}

#[tauri::command]
fn sidecar_send(
    command: sidecar::SidecarCommand,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let guard = state.sidecar.lock().map_err(|_| "sidecar lock poisoned")?;
    guard
        .as_ref()
        .ok_or_else(|| "sidecar is not running".to_string())?
        .send(&command)
        .map_err(|e| e.to_string())
}

/// Which System Settings privacy pane to open. Mirrored by `PrivacyPane` in
/// `src/types.ts`.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyPane {
    Microphone,
    ScreenRecording,
}

impl PrivacyPane {
    /// Anchors into the Privacy & Security pane. Deep-linking matters: the
    /// Screen Recording list is several scrolls down and users reliably fail to
    /// find it from a generic "open Settings".
    fn url(self) -> &'static str {
        match self {
            PrivacyPane::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            PrivacyPane::ScreenRecording => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
        }
    }
}

#[tauri::command]
fn open_privacy_settings(pane: PrivacyPane) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(pane.url())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open System Settings: {e}"))
}

/// Kills the child process without stopping supervision, so the restart path
/// can be watched happening rather than only asserted in a test.
#[tauri::command]
fn sidecar_simulate_crash(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let guard = state.sidecar.lock().map_err(|_| "sidecar lock poisoned")?;
    guard
        .as_ref()
        .ok_or_else(|| "sidecar is not running".to_string())?
        .kill_child()
        .map_err(|e| e.to_string())
}

/// Result of [`db_selftest`]. Mirrored by `DbSelftest` in `src/types.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbSelftest {
    pub schema_version: i32,
    pub db_path: String,
    pub stats: repo::DbStats,
    /// Text found by the FTS index when searching a stemmed variant.
    pub fts_hit: Option<String>,
    /// Owner id of the nearest vector to the probe.
    pub vector_hit: Option<String>,
}

/// Exercises the real data layer end to end and reports what came back.
///
/// Runs against a scratch in-memory database, not the user's, so clicking it
/// repeatedly can't pollute real meetings. It proves migrations, FTS5 stemming,
/// and sqlite-vec nearest-neighbour all work *on this machine* — the parts most
/// likely to break on a different SQLite build.
#[tauri::command]
fn db_selftest(state: tauri::State<'_, AppState>) -> Result<DbSelftest, String> {
    let scratch = Database::open_in_memory().map_err(|e| e.to_string())?;
    let conn = scratch.connection();

    repo::insert_meeting(conn, "selftest", "Selftest meeting", 0).map_err(|e| e.to_string())?;
    repo::insert_utterance(
        conn,
        "selftest",
        0,
        "system",
        "So the deadline for the migration is the fourteenth.",
        12_400,
        14_700,
        Some(0.93),
    )
    .map_err(|e| e.to_string())?;
    repo::insert_note_block(conn, "selftest", "b0", 0, "deadline = 14th", Some(16_000))
        .map_err(|e| e.to_string())?;

    // "migrate" must match "migration" — proves the porter tokenizer is active.
    let fts_hit = repo::search_utterances(conn, "migrate", 1)
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .map(|hit| hit.text);

    let mut probe = vec![0.0f32; db::EMBEDDING_DIM];
    probe[0] = 1.0;
    let mut other = vec![0.0f32; db::EMBEDDING_DIM];
    other[1] = 1.0;
    repo::insert_embedding(conn, "utterance", "near", &probe).map_err(|e| e.to_string())?;
    repo::insert_embedding(conn, "utterance", "far", &other).map_err(|e| e.to_string())?;
    let vector_hit = repo::nearest_embeddings(conn, &probe, 1)
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .map(|hit| hit.owner_id);

    // Stats come from the user's real database, so the card shows actual
    // content alongside the synthetic checks.
    let real = state
        .db
        .lock()
        .map_err(|_| "db lock poisoned".to_string())?;
    let stats = repo::stats(real.connection()).map_err(|e| e.to_string())?;

    Ok(DbSelftest {
        schema_version: real.schema_version().map_err(|e| e.to_string())?,
        db_path: state.db_path.clone(),
        stats,
        fts_hit,
        vector_hit,
    })
}

/// The app's compiled-in config, assets, and resolved ACL.
///
/// `generate_context!` embeds an `_EMBED_INFO_PLIST` symbol, so it may expand
/// exactly once per crate. Funnelling it through one generic function lets both
/// `run()` and the tests share it — and means the tests exercise the *real*
/// capability file rather than an empty mock ACL that would allow everything or
/// nothing.
fn app_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("oatmeal.sqlite");

            let db = Database::open(&path)?;
            // Any meeting still marked `recording` belongs to a process that is
            // gone. Close it out before the UI can see it.
            match repo::recover_interrupted_meetings(db.connection(), now_ms()) {
                Ok(0) => {}
                Ok(n) => eprintln!("recovered {n} interrupted meeting(s)"),
                Err(err) => eprintln!("meeting recovery failed: {err}"),
            }
            app.manage(AppState {
                db: Mutex::new(db),
                db_path: path.to_string_lossy().into_owned(),
                sidecar: Mutex::new(None),
                meeting: Mutex::new(MeetingState::default()),
                last_permissions: Mutex::new(None),
            });

            if std::env::var(AUTOSTART_ENV).as_deref() == Ok("1") {
                let handle = app.handle().clone();
                // Deferred: the webview has to be listening before events fire,
                // or the log renders empty and the harness looks broken.
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    if let Err(err) = spawn_sidecar(&handle, true) {
                        eprintln!("harness autostart failed: {err}");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health_check,
            db_selftest,
            sidecar_start,
            sidecar_stop,
            sidecar_send,
            sidecar_simulate_crash,
            open_privacy_settings,
            permissions_snapshot,
            meeting_start,
            meeting_stop,
            meeting_active,
            meetings_list,
            meeting_transcript,
            notes_save,
            notes_load,
            meeting_state,
            meeting_rename,
            meeting_delete
        ])
        .run(app_context())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_info_reports_the_crate_version() {
        let info = HealthInfo::current();
        assert_eq!(info.app_version, env!("CARGO_PKG_VERSION"));
        assert!(!info.arch.is_empty());
        assert!(!info.os.is_empty());
    }

    #[test]
    fn health_info_serializes_camel_case_for_the_frontend() {
        let info = HealthInfo::current();
        let json = serde_json::to_value(&info).unwrap();
        // The TS interface reads these exact keys; a rename here breaks the UI
        // silently, so pin them.
        assert!(json.get("appVersion").is_some());
        assert!(json.get("buildProfile").is_some());
        assert!(json.get("arch").is_some());
        assert!(json.get("os").is_some());
    }

    /// Drives `health_check` through the real IPC dispatcher against a mock
    /// runtime. Unit-testing `HealthInfo::current()` only proves the struct is
    /// right; this proves the command is actually registered and reachable the
    /// way the webview reaches it.
    ///
    /// The `url` must be `tauri://localhost`. Tauri gates app commands with
    /// `(is_plugin_command || has_app_acl_manifest || !is_local)`, so any URL it
    /// doesn't recognise as local is treated as remote web content and rejected
    /// for lacking an explicit remote capability — which reads like a broken
    /// command rather than a wrong test origin.
    #[test]
    fn health_check_round_trips_over_ipc() {
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![health_check])
            .build(app_context())
            .expect("failed to build mock app");

        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("failed to build mock webview");

        let response = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "health_check".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        )
        .expect("health_check errored over IPC");

        let value: serde_json::Value = response.deserialize().expect("bad IPC payload");
        assert_eq!(value["appVersion"], env!("CARGO_PKG_VERSION"));
        assert!(value["os"].as_str().is_some_and(|s| !s.is_empty()));
    }

    /// A command the frontend never registered must not be silently reachable.
    #[test]
    fn unregistered_command_is_rejected() {
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![health_check])
            .build(app_context())
            .expect("failed to build mock app");

        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("failed to build mock webview");

        let result = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "definitely_not_a_command".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        );

        assert!(result.is_err(), "unknown command should not resolve");
    }
}
