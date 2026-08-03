pub mod chat;
pub mod db;
pub mod detect;
pub mod embed;
pub mod link;
pub mod llm;
pub mod meeting;
pub mod panel;
pub mod search;
pub mod sidecar;
pub mod tray;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use db::{repo, Database};
use llm::keys::{KeyStore, Keychain};
use llm::provider::{ProviderConfig, ProviderKind};
use llm::LlmClient;
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
    /// Chosen LLM provider. Holds no key — those live in the Keychain and are
    /// fetched at request time.
    pub provider: Mutex<ProviderConfig>,
    pub keys: Box<dyn KeyStore>,
    pub llm: LlmClient,
    /// The `llama-server` Oatmeal can run itself, for the no-key offline path.
    pub runtime: llm::bundled::Runtime,
    /// Shared HTTP client for downloads, so connection pooling survives across
    /// a server fetch followed immediately by a multi-gigabyte model fetch.
    pub http: reqwest::Client,
    /// Set to ask an in-flight download to stop. Reset when one starts.
    pub cancel_download: Arc<AtomicBool>,
    /// Wall clock at which the current recording began, for the menu bar timer.
    /// `None` whenever nothing is being recorded.
    pub recording_since: Mutex<Option<i64>>,
    /// Candidates waiting to be offered (G22).
    pub candidates: Mutex<detect::Queue>,
    /// Calendar events already offered, so a five-minute poll does not re-raise
    /// a meeting the user has dismissed.
    pub offered_events: Mutex<Vec<String>>,
    /// An app awaiting its one-time "always or never" answer.
    ///
    /// Held rather than only emitted, for the same reason permissions are
    /// cached: the popup window is created *by* the same code that raises the
    /// question, so it cannot possibly be listening yet. An event alone would
    /// arrive before anything was there to hear it, and the window would open
    /// blank.
    pub pending_question: Mutex<Option<AppQuestion>>,
    /// Last permission snapshot the sidecar reported.
    ///
    /// Cached because permissions arrive as a one-shot event: anything that
    /// subscribes afterwards (a later mount, a hot reload, a second window)
    /// would otherwise show "unknown" forever despite the answer being known.
    pub last_permissions: Mutex<Option<PermissionsSnapshot>>,
    /// Live linker weights. Mutable at runtime so the tuning panel can move
    /// alpha/beta and re-link without a rebuild — the defaults are measured
    /// (see `link::eval`), not sacred.
    pub link_params: Mutex<link::LinkParams>,
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
    // The lock has to go before indexing starts, or the background pass blocks
    // on a guard this thread is still holding.
    drop(db);

    // Linking is deferred rather than awaited: it can take seconds on a long
    // meeting, and nothing the user does next depends on it having finished.
    // A failure here is logged, never fatal — the transcript is already safe.
    let handle = app.clone();
    let id = meeting_id.clone();
    tauri::async_runtime::spawn(async move {
        match index_meeting_now(&handle, &id).await {
            Ok(report) => {
                if let Some(reason) = &report.degraded {
                    eprintln!("linked {id} on timestamps alone ({reason})");
                }
                let _ = handle.emit("meeting://indexed", &report);
            }
            Err(err) => eprintln!("failed to index {id}: {err}"),
        }
    });
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
fn meeting_stop(app: tauri::AppHandle) -> Result<(), String> {
    // Goes through `transition` so the frontend hears `processing`. Mutating the
    // machine directly here left the UI showing "recording" until the sidecar
    // confirmed, which hid the finalising step entirely.
    transition(&app, MeetingEvent::StopRequested)?;

    let state = app.state::<AppState>();
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

    // The tray mirrors the machine rather than the record button, for the same
    // reason the frontend does: a meeting can start from the hotkey, the menu,
    // or detection, and a menu bar that only knows about one of them lies.
    if let Ok(mut since) = state.recording_since.lock() {
        *since = match (next_state.is_recording(), *since) {
            (true, Some(started)) => Some(started),
            (true, None) => Some(now_ms()),
            (false, _) => None,
        };
    }
    refresh_tray(app);

    let _ = app.emit(MEETING_STATE_EVENT, &next_state);
    Ok(next_state)
}

/// The global start/stop shortcut: ⌃⌥⌘R.
///
/// Three modifiers on purpose. This is registered system-wide, so it takes the
/// key away from every other app — a two-modifier combination would eventually
/// shadow something the user cares about more than this.
pub fn record_shortcut() -> tauri_plugin_global_shortcut::Shortcut {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
    Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER),
        Code::KeyR,
    )
}

// ------------------------------------------------------------ folders + search

#[tauri::command]
fn folders_list(state: tauri::State<'_, AppState>) -> Result<Vec<repo::Folder>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::list_folders(db.connection()).map_err(|e| e.to_string())
}

#[tauri::command]
fn folder_create(
    state: tauri::State<'_, AppState>,
    name: String,
    parent_id: Option<String>,
) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("a folder needs a name".into());
    }
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::create_folder(db.connection(), &name, parent_id.as_deref(), now_ms())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn folder_rename(
    state: tauri::State<'_, AppState>,
    folder_id: String,
    name: String,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a folder needs a name".into());
    }
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::rename_folder(db.connection(), &folder_id, &name).map_err(|e| e.to_string())
}

/// Deletes a folder. Its meetings survive and become unfiled.
#[tauri::command]
fn folder_delete(state: tauri::State<'_, AppState>, folder_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::delete_folder(db.connection(), &folder_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn meeting_set_folder(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    folder_id: Option<String>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::set_meeting_folder(db.connection(), &meeting_id, folder_id.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn folder_meetings(
    state: tauri::State<'_, AppState>,
    folder_id: Option<String>,
) -> Result<Vec<repo::MeetingSummary>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::meetings_in_folder(db.connection(), folder_id.as_deref(), 200).map_err(|e| e.to_string())
}

/// Searches transcripts. Blocking thread for the same reason indexing is:
/// `Connection` is `!Send`, so its guard cannot be held across an `.await`.
#[tauri::command]
async fn search_transcripts(
    app: tauri::AppHandle,
    query: String,
    folder_id: Option<String>,
) -> Result<search::SearchResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let embedder = embed::HttpEmbedder::local();
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        tauri::async_runtime::block_on(search::search(
            db.connection(),
            &query,
            folder_id.as_deref(),
            &embedder,
            25,
        ))
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("search thread failed: {e}"))?
}

// --------------------------------------------------------------------- chat

/// Answers a question over one meeting or a whole folder.
#[tauri::command]
async fn chat_ask(
    app: tauri::AppHandle,
    question: String,
    meeting_id: Option<String>,
    folder_id: Option<String>,
) -> Result<chat::ChatReply, String> {
    // Retrieval touches the database and cannot cross an await; generation is a
    // network call and must not hold the lock. So they are split: gather under
    // the lock, release, then ask.
    let gather = {
        let app = app.clone();
        let question = question.clone();
        let meeting_id = meeting_id.clone();
        let folder_id = folder_id.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<AppState>();
            let embedder = embed::HttpEmbedder::local();
            let db = state.db.lock().map_err(|_| "db lock poisoned")?;
            tauri::async_runtime::block_on(chat::gather_context(
                db.connection(),
                &question,
                meeting_id.as_deref(),
                folder_id.as_deref(),
                &embedder,
                12,
            ))
            .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("retrieval thread failed: {e}"))?
    }?;

    let state = app.state::<AppState>();
    let config = state
        .provider
        .lock()
        .map_err(|_| "provider lock poisoned")?
        .clone();

    chat::ask(&state.llm, &config, state.keys.as_ref(), &question, gather)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- detection

/// An app waiting on its one-time answer. Mirrored by `AppQuestion` in
/// `src/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppQuestion {
    pub bundle_id: String,
    pub app_name: Option<String>,
}

/// Settings keys. Strings because `settings` is a key/value table; collected
/// here so a typo is a compile error at one site rather than a silent default.
pub const SETTING_LEAD_MS: &str = "detect.lead_ms";
pub const SETTING_MIC_ENABLED: &str = "detect.mic_enabled";
pub const SETTING_CALENDAR_ENABLED: &str = "detect.calendar_enabled";

/// Detection settings as the UI sees them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionSettings {
    /// How long before a calendar event to offer. Default 90s, per SPEC.
    pub lead_ms: i64,
    pub mic_enabled: bool,
    pub calendar_enabled: bool,
}

impl Default for DetectionSettings {
    fn default() -> Self {
        Self {
            lead_ms: 90_000,
            // Both off until the user opts in. Detection watches what other
            // apps are doing and reads the calendar; neither should start
            // happening because the app was launched.
            mic_enabled: false,
            calendar_enabled: false,
        }
    }
}

pub fn load_settings(conn: &rusqlite::Connection) -> DetectionSettings {
    let defaults = DetectionSettings::default();
    let read = |key: &str| repo::get_setting(conn, key).ok().flatten();
    DetectionSettings {
        lead_ms: read(SETTING_LEAD_MS)
            .and_then(|v| v.parse().ok())
            .unwrap_or(defaults.lead_ms),
        mic_enabled: read(SETTING_MIC_ENABLED)
            .map(|v| v == "1")
            .unwrap_or(defaults.mic_enabled),
        calendar_enabled: read(SETTING_CALENDAR_ENABLED)
            .map(|v| v == "1")
            .unwrap_or(defaults.calendar_enabled),
    }
}

/// The user's stored per-app rules, keyed by bundle id.
fn stored_rules(conn: &rusqlite::Connection) -> HashMap<String, detect::RuleMode> {
    repo::detection_rules(conn)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|rule| detect::RuleMode::parse(&rule.mode).map(|mode| (rule.bundle_id, mode)))
        .collect()
}

/// An app started using the microphone.
fn on_mic_started(app: &tauri::AppHandle, apps: &[sidecar::MicApp]) {
    let state = app.state::<AppState>();

    // Never offer while already recording. The user is in a meeting; a popup
    // asking whether they would like to record one is noise at best.
    if state
        .meeting
        .lock()
        .map(|m| m.is_capturing())
        .unwrap_or(false)
    {
        return;
    }

    let Ok(db) = state.db.lock() else { return };
    let settings = load_settings(db.connection());
    if !settings.mic_enabled {
        return;
    }
    let rules = stored_rules(db.connection());
    drop(db);

    for mic_app in apps {
        let Some(bundle_id) = mic_app.bundle_id.as_deref() else {
            continue;
        };
        match detect::rules::decide(bundle_id, &rules) {
            detect::Decision::Ignore => {}
            detect::Decision::Ask => {
                // One question, then remembered forever. Asked through the same
                // popup so there is only ever one kind of interruption.
                let question = AppQuestion {
                    bundle_id: bundle_id.to_string(),
                    app_name: mic_app.name.clone(),
                };
                if let Ok(mut pending) = state.pending_question.lock() {
                    *pending = Some(question.clone());
                }
                // Stored first, then announced, then the window is opened. The
                // window cannot be listening yet, so it reads the stored value
                // on mount; the event is only for a window already open.
                let _ = app.emit("detect://ask", &question);
                show_popup(app);
            }
            detect::Decision::Offer => {
                let candidate = detect::Candidate {
                    id: uuid::Uuid::new_v4().to_string(),
                    source: detect::Source::Mic,
                    title: None,
                    bundle_id: Some(bundle_id.to_string()),
                    app_name: mic_app.name.clone(),
                    calendar_event_id: None,
                    at_ms: now_ms(),
                };
                offer_candidate(app, candidate);
            }
        }
    }
}

/// The calendar reported its upcoming window.
fn on_calendar_events(app: &tauri::AppHandle, events: &[detect::CalendarEvent]) {
    let state = app.state::<AppState>();
    if state
        .meeting
        .lock()
        .map(|m| m.is_capturing())
        .unwrap_or(false)
    {
        return;
    }

    let Ok(db) = state.db.lock() else { return };
    let settings = load_settings(db.connection());
    drop(db);
    if !settings.calendar_enabled {
        return;
    }

    let already = state
        .offered_events
        .lock()
        .map(|o| o.clone())
        .unwrap_or_default();
    let due = detect::calendar::due(events, now_ms(), settings.lead_ms, &already);

    for event in due {
        if let Ok(mut offered) = state.offered_events.lock() {
            offered.push(event.id.clone());
        }
        offer_candidate(
            app,
            detect::Candidate {
                id: uuid::Uuid::new_v4().to_string(),
                source: detect::Source::Calendar,
                title: event.title.clone(),
                bundle_id: None,
                app_name: None,
                calendar_event_id: Some(event.id.clone()),
                at_ms: now_ms(),
            },
        );
    }
}

/// Queues a candidate and shows the popup.
///
/// Dedup happens in the queue, so a calendar event and a mic activation for the
/// same call raise the popup once with the better-informed of the two.
fn offer_candidate(app: &tauri::AppHandle, candidate: detect::Candidate) {
    let state = app.state::<AppState>();
    let Ok(mut queue) = state.candidates.lock() else {
        return;
    };
    queue.offer(candidate);
    let pending = queue.pending().to_vec();
    drop(queue);

    let _ = app.emit("detect://candidates", &pending);
    show_popup(app);
}

/// The floating offer window.
///
/// A separate always-on-top window rather than something inside the main one:
/// the whole point is to be seen while the user is in a video call, looking at
/// anything but Oatmeal.
pub fn show_popup(app: &tauri::AppHandle) {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    if let Some(existing) = app.get_webview_window("popup") {
        let _ = existing.show();
        let _ = existing.set_focus();
        return;
    }

    let built = WebviewWindowBuilder::new(app, "popup", WebviewUrl::App("index.html".into()))
        .title("Oatmeal")
        .inner_size(360.0, 190.0)
        .resizable(false)
        .always_on_top(true)
        // No traffic lights: this is a transient offer, and a close button
        // invites dismissing it in a way that records no decision.
        .decorations(false)
        .skip_taskbar(true)
        .focused(false)
        .build();

    if let Err(err) = built {
        eprintln!("could not show the detection popup: {err}");
    }
}

pub fn hide_popup(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("popup") {
        let _ = window.close();
    }
}

/// Applies the user's answer to an offer.
#[tauri::command]
fn detection_respond(
    app: tauri::AppHandle,
    candidate_id: String,
    outcome: detect::Outcome,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let candidate = state
        .candidates
        .lock()
        .map_err(|_| "candidate lock poisoned")?
        .resolve(&candidate_id);

    let Some(candidate) = candidate else {
        // Already answered, or expired. Not an error: two clicks on a popup
        // that was closing is an ordinary race.
        hide_popup(&app);
        return Ok(None);
    };

    match outcome {
        detect::Outcome::Start => {
            hide_popup(&app);
            let id = meeting_start(app.clone(), candidate.title.clone())?;
            return Ok(Some(id));
        }
        detect::Outcome::IgnoreApp => {
            // "Never ask about this app again" — the permanent half of the
            // one-time question.
            if let Some(bundle_id) = &candidate.bundle_id {
                let db = state.db.lock().map_err(|_| "db lock poisoned")?;
                repo::set_detection_rule(
                    db.connection(),
                    bundle_id,
                    candidate.app_name.as_deref(),
                    detect::RuleMode::Ignore.as_str(),
                    now_ms(),
                )
                .map_err(|e| e.to_string())?;
            }
        }
        detect::Outcome::Ignore | detect::Outcome::Expired => {}
    }

    let remaining = state
        .candidates
        .lock()
        .map(|q| q.pending().to_vec())
        .unwrap_or_default();
    let _ = app.emit("detect://candidates", &remaining);
    if remaining.is_empty() {
        hide_popup(&app);
    }
    Ok(None)
}

/// Records the answer to "should this app be allowed to offer?".
#[tauri::command]
fn detection_answer_app(
    app: tauri::AppHandle,
    bundle_id: String,
    app_name: Option<String>,
    allow: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::set_detection_rule(
        db.connection(),
        &bundle_id,
        app_name.as_deref(),
        if allow {
            detect::RuleMode::Allow.as_str()
        } else {
            detect::RuleMode::Ignore.as_str()
        },
        now_ms(),
    )
    .map_err(|e| e.to_string())?;
    drop(db);

    if let Ok(mut pending) = state.pending_question.lock() {
        *pending = None;
    }
    hide_popup(&app);
    Ok(())
}

/// The app awaiting an answer, if any. Read by the popup on mount.
#[tauri::command]
fn detection_pending_question(state: tauri::State<'_, AppState>) -> Option<AppQuestion> {
    state.pending_question.lock().ok().and_then(|q| q.clone())
}

#[tauri::command]
fn detection_candidates(state: tauri::State<'_, AppState>) -> Vec<detect::Candidate> {
    state
        .candidates
        .lock()
        .map(|q| q.pending().to_vec())
        .unwrap_or_default()
}

#[tauri::command]
fn detection_settings(state: tauri::State<'_, AppState>) -> Result<DetectionSettings, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    Ok(load_settings(db.connection()))
}

/// Stores settings and starts or stops the watchers to match.
#[tauri::command]
fn detection_set_settings(
    app: tauri::AppHandle,
    settings: DetectionSettings,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    {
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        let conn = db.connection();
        let write = |key: &str, value: String| {
            let _ = repo::set_setting(conn, key, &value);
        };
        write(SETTING_LEAD_MS, settings.lead_ms.to_string());
        write(
            SETTING_MIC_ENABLED,
            if settings.mic_enabled { "1" } else { "0" }.into(),
        );
        write(
            SETTING_CALENDAR_ENABLED,
            if settings.calendar_enabled { "1" } else { "0" }.into(),
        );
    }

    apply_detection_settings(&app, &settings);
    Ok(())
}

/// Tells the sidecar which watchers should be running.
///
/// A no-op when the sidecar is not up — the settings are stored either way, and
/// they are re-applied on the next spawn.
pub fn apply_detection_settings(app: &tauri::AppHandle, settings: &DetectionSettings) {
    let state = app.state::<AppState>();
    let Ok(guard) = state.sidecar.lock() else {
        return;
    };
    let Some(supervisor) = guard.as_ref() else {
        return;
    };
    let _ = supervisor.send(&sidecar::SidecarCommand::WatchMic {
        enabled: settings.mic_enabled,
    });
    let _ = supervisor.send(&sidecar::SidecarCommand::WatchCalendar {
        enabled: settings.calendar_enabled,
        // Requesting access is a user-visible prompt, so it only happens when
        // they have just switched the calendar on.
        request: settings.calendar_enabled,
    });

    if !settings.mic_enabled && !settings.calendar_enabled {
        // Nothing may surface from a queue built under the old settings.
        if let Ok(mut queue) = state.candidates.lock() {
            queue.clear();
        }
    }
}

#[tauri::command]
fn detection_rules_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<repo::DetectionRule>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::detection_rules(db.connection()).map_err(|e| e.to_string())
}

#[tauri::command]
fn detection_rule_clear(
    state: tauri::State<'_, AppState>,
    bundle_id: String,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::clear_detection_rule(db.connection(), &bundle_id).map_err(|e| e.to_string())
}

/// The shipped allowlist, so settings can show what is on by default.
#[tauri::command]
fn detection_builtin_apps() -> Vec<(String, String)> {
    detect::rules::BUILTIN_ALLOW
        .iter()
        .map(|(id, name)| ((*id).to_string(), (*name).to_string()))
        .collect()
}

// ----------------------------------------------------------------- menu bar

/// Rebuilds the tray menu, title and tooltip from current state.
pub fn refresh_tray(app: &tauri::AppHandle) {
    use tauri::tray::TrayIconId;

    let Some(icon) = app.tray_by_id(&TrayIconId::new("oatmeal")) else {
        return;
    };
    let state = app.state::<AppState>();

    let (recording, processing) = match state.meeting.lock() {
        Ok(machine) => (
            machine.is_recording(),
            machine.is_capturing() && !machine.is_recording(),
        ),
        Err(_) => (false, false),
    };
    let elapsed = state
        .recording_since
        .lock()
        .ok()
        .and_then(|since| *since)
        .map(|started| now_ms() - started)
        .unwrap_or(0);

    let recent: Vec<tray::RecentEntry> = state
        .db
        .lock()
        .ok()
        .and_then(|db| repo::list_meetings(db.connection(), tray::RECENT_LIMIT).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|meeting| tray::RecentEntry {
            label: tray::menu_label(meeting.title.as_deref().unwrap_or_default(), 34),
            id: meeting.id,
        })
        .collect();

    if let Ok(menu) = tray::build_menu(app, recording, &recent) {
        let _ = icon.set_menu(Some(menu));
    }
    let _ = icon.set_title(Some(tray::tray_title(recording, elapsed)));
    let _ = icon.set_tooltip(Some(tray::tray_tooltip(recording, processing)));
}

/// Runs a tray menu choice.
pub fn on_tray_action(app: &tauri::AppHandle, action: tray::TrayAction) {
    match action {
        tray::TrayAction::StartRecording => {
            if let Err(err) = meeting_start(app.clone(), None) {
                eprintln!("tray could not start a recording: {err}");
            }
        }
        tray::TrayAction::StopRecording => {
            if let Err(err) = meeting_stop(app.clone()) {
                eprintln!("tray could not stop the recording: {err}");
            }
        }
        tray::TrayAction::OpenWindow => tray::focus_main(app),
        tray::TrayAction::OpenMeeting(id) => {
            // The window has to be up before the event fires, or a hidden
            // window misses it and opens on the wrong meeting.
            tray::focus_main(app);
            let _ = app.emit("meeting://open", id);
        }
        tray::TrayAction::Quit => {
            // Stop cleanly rather than exiting under a live recording: the
            // sidecar owns an unfinalised audio file, and killing the process
            // leaves it unreadable.
            let recording = app
                .state::<AppState>()
                .meeting
                .lock()
                .map(|m| m.is_recording())
                .unwrap_or(false);
            if recording {
                let _ = meeting_stop(app.clone());
            }
            app.exit(0);
        }
        tray::TrayAction::Ignore => {}
    }
}

/// Toggles recording. What the global hotkey and "Record now" both mean.
pub fn toggle_recording(app: &tauri::AppHandle) {
    let recording = app
        .state::<AppState>()
        .meeting
        .lock()
        .map(|m| m.is_recording())
        .unwrap_or(false);

    on_tray_action(
        app,
        if recording {
            tray::TrayAction::StopRecording
        } else {
            tray::TrayAction::StartRecording
        },
    );
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

            // Detection (Phase 5). Both are reports; whether anything happens
            // is decided in `detect`, and nothing here ever starts a recording.
            match inner {
                sidecar::SidecarEvent::MicActivity { started, .. } if !started.is_empty() => {
                    on_mic_started(&app_handle, started);
                }
                sidecar::SidecarEvent::CalendarEvents { events, .. } => {
                    on_calendar_events(&app_handle, events);
                }
                _ => {}
            }
        }

        // The watchers live in the sidecar, so a restart loses them. Re-apply
        // the stored settings on every `ready` — including after a crash — or
        // detection would silently stop working until the app was relaunched.
        if let SupervisorEvent::Event {
            event: sidecar::SidecarEvent::Ready { .. },
        } = &event
        {
            let settings = app_handle
                .state::<AppState>()
                .db
                .lock()
                .map(|db| load_settings(db.connection()))
                .unwrap_or_default();
            let _ = for_callback.send(&sidecar::SidecarCommand::WatchMic {
                enabled: settings.mic_enabled,
            });
            let _ = for_callback.send(&sidecar::SidecarCommand::WatchCalendar {
                enabled: settings.calendar_enabled,
                // Never prompt from a restart — only when the user just enabled it.
                request: false,
            });
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
                    let meeting_id = handle
                        .state::<AppState>()
                        .meeting
                        .lock()
                        .ok()
                        .and_then(|m| m.active_meeting().map(str::to_string));

                    if let Err(err) = meeting_stop(handle.clone()) {
                        eprintln!("harness: could not stop meeting: {err}");
                    }

                    if std::env::var("OATMEAL_HARNESS_GENERATE").as_deref() == Ok("1") {
                        // Give the sidecar time to flush its last utterances.
                        std::thread::sleep(std::time::Duration::from_secs(20));
                        if let Some(id) = meeting_id {
                            let handle = handle.clone();
                            tauri::async_runtime::spawn(async move {
                                match panel_generate(handle, id, "default".into()).await {
                                    Ok(panel) => {
                                        eprintln!("harness: generated panel {}", panel.id)
                                    }
                                    Err(err) => {
                                        eprintln!("harness: panel generation failed: {err}")
                                    }
                                }
                            });
                        }
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

// -------------------------------------------------------------------- linking

/// Embeds and links a meeting, storing the result.
///
/// Exposed as a command as well as running automatically on completion, because
/// the tuning panel needs to re-link on demand after the weights move.
#[tauri::command]
async fn meeting_index(
    app: tauri::AppHandle,
    meeting_id: String,
) -> Result<link::pipeline::IndexReport, String> {
    index_meeting_now(&app, &meeting_id).await
}

/// Shared by the command and the automatic post-meeting run.
///
/// Runs on a blocking thread rather than a async worker. `Connection` and its
/// `MutexGuard` are both `!Send`, so the lock cannot be held across an `.await`
/// in an async command — but it can be held for the whole of a synchronous
/// closure that drives the future to completion itself.
async fn index_meeting_now(
    app: &tauri::AppHandle,
    meeting_id: &str,
) -> Result<link::pipeline::IndexReport, String> {
    let app = app.clone();
    let meeting_id = meeting_id.to_string();

    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let params = *state
            .link_params
            .lock()
            .map_err(|_| "link params lock poisoned")?;

        let embedder = embed::HttpEmbedder::local();
        let mut db = state.db.lock().map_err(|_| "db lock poisoned")?;
        tauri::async_runtime::block_on(link::pipeline::index_meeting(
            db.connection_mut(),
            &meeting_id,
            &embedder,
            &params,
        ))
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("indexing thread failed: {e}"))?
}

#[tauri::command]
fn meeting_links(
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<repo::StoredLink>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::meeting_links(db.connection(), &meeting_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn link_params_get(state: tauri::State<'_, AppState>) -> Result<link::LinkParams, String> {
    Ok(*state
        .link_params
        .lock()
        .map_err(|_| "link params lock poisoned")?)
}

/// Replaces the linker weights. Does not re-link — the caller decides which
/// meeting to recompute, so moving a slider does not silently rewrite the
/// entire library.
#[tauri::command]
fn link_params_set(
    params: link::LinkParams,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    *state
        .link_params
        .lock()
        .map_err(|_| "link params lock poisoned")? = params;
    Ok(())
}

// ------------------------------------------------------------------ providers

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub kind: ProviderKind,
    pub label: String,
    pub default_base_url: String,
    pub default_model: String,
    pub requires_key: bool,
    pub is_local: bool,
    /// Whether a key is already stored. Never the key itself.
    pub has_key: bool,
}

#[tauri::command]
fn providers_list(state: tauri::State<'_, AppState>) -> Vec<ProviderInfo> {
    ProviderKind::all()
        .iter()
        .map(|kind| ProviderInfo {
            kind: *kind,
            label: kind.label().to_string(),
            default_base_url: kind.default_base_url().to_string(),
            default_model: kind.default_model().to_string(),
            requires_key: kind.requires_key(),
            is_local: kind.is_local(),
            has_key: ProviderConfig::preset(*kind)
                .keychain_ref
                .map(|r| state.keys.has(&r))
                .unwrap_or(false),
        })
        .collect()
}

#[tauri::command]
fn provider_current(state: tauri::State<'_, AppState>) -> Result<ProviderConfig, String> {
    Ok(state.provider.lock().map_err(|_| "lock poisoned")?.clone())
}

#[tauri::command]
fn provider_select(
    kind: ProviderKind,
    model: Option<String>,
    base_url: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ProviderConfig, String> {
    let mut config = ProviderConfig::preset(kind);
    if let Some(model) = model.filter(|m| !m.trim().is_empty()) {
        config.model = model.trim().to_string();
    }
    if let Some(url) = base_url.filter(|u| !u.trim().is_empty()) {
        config.base_url = url.trim().to_string();
    }
    *state.provider.lock().map_err(|_| "lock poisoned")? = config.clone();
    Ok(config)
}

/// Stores an API key in the Keychain. The key is never returned, logged, or
/// written to the database.
#[tauri::command]
fn provider_set_key(
    kind: ProviderKind,
    key: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let reference = ProviderConfig::preset(kind)
        .keychain_ref
        .ok_or_else(|| format!("{} does not use an API key", kind.label()))?;

    if key.trim().is_empty() {
        return state.keys.delete(&reference).map_err(|e| e.to_string());
    }
    state
        .keys
        .set(&reference, key.trim())
        .map_err(|e| e.to_string())
}

/// Round-trips a tiny prompt so a misconfiguration surfaces here rather than
/// when someone is waiting on a summary.
#[tauri::command]
async fn provider_test(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let config = state.provider.lock().map_err(|_| "lock poisoned")?.clone();

    let request = llm::provider::ChatRequest::new(vec![llm::provider::Message::user(
        "Reply with the single word: ready",
    )]);

    let reply = state
        .llm
        .chat(&config, &request, state.keys.as_ref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(reply.trim().chars().take(120).collect())
}

/// State of the local model runtime.
#[tauri::command]
fn runtime_state(state: tauri::State<'_, AppState>) -> llm::bundled::RuntimeState {
    state.runtime.state()
}

#[tauri::command]
fn runtime_models() -> Vec<llm::bundled::ModelOption> {
    llm::bundled::model_options()
}

/// Starts the local model server, so the fully-offline path needs no terminal.
#[tauri::command]
fn runtime_start(state: tauri::State<'_, AppState>) -> Result<u32, String> {
    state.runtime.start()
}

#[tauri::command]
fn runtime_stop(state: tauri::State<'_, AppState>) {
    state.runtime.stop();
}

/// What is on disk for each curated model, so the picker can offer
/// download / resume / installed rather than one undifferentiated button.
#[tauri::command]
fn runtime_model_status(
    state: tauri::State<'_, AppState>,
) -> Vec<(String, llm::bundled::ModelStatus)> {
    llm::bundled::model_options()
        .into_iter()
        .map(|option| {
            let status = state.runtime.model_status(&option.id);
            (option.id, status)
        })
        .collect()
}

/// Downloads `llama-server`. Progress arrives on `runtime://download`.
#[tauri::command]
async fn runtime_install_server(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.cancel_download.store(false, Ordering::SeqCst);

    let emitter = app.clone();
    let cancel = state.cancel_download.clone();
    state
        .runtime
        .install_server(
            &state.http,
            move |progress| {
                let _ = emitter.emit("runtime://download", &progress);
            },
            move || cancel.load(Ordering::SeqCst),
        )
        .await
}

/// Downloads a model. Multi-gigabyte, so it reports progress and can be
/// cancelled; a cancelled download keeps its bytes and resumes on retry.
#[tauri::command]
async fn runtime_install_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.cancel_download.store(false, Ordering::SeqCst);

    let emitter = app.clone();
    let cancel = state.cancel_download.clone();
    state
        .runtime
        .install_model(
            &state.http,
            &model_id,
            move |progress| {
                let _ = emitter.emit("runtime://download", &progress);
            },
            move || cancel.load(Ordering::SeqCst),
        )
        .await
}

/// Asks the running download to stop. The partial file is kept deliberately.
#[tauri::command]
fn runtime_cancel_download(state: tauri::State<'_, AppState>) {
    state.cancel_download.store(true, Ordering::SeqCst);
}

// -------------------------------------------------------------------- panels

#[tauri::command]
fn templates_list() -> Vec<panel::Template> {
    panel::prompt::builtin_templates()
}

#[tauri::command]
fn panels_list(
    meeting_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<repo::Panel>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::meeting_panels(db.connection(), &meeting_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn panel_delete(panel_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::delete_panel(db.connection(), &panel_id).map_err(|e| e.to_string())
}

/// Generates a panel and stores it.
///
/// Always inserts a new row: regenerating forks rather than overwrites, so an
/// earlier panel the user liked is never destroyed by a retry.
#[tauri::command]
async fn panel_generate(
    app: tauri::AppHandle,
    meeting_id: String,
    template_id: String,
) -> Result<repo::Panel, String> {
    let state = app.state::<AppState>();

    let template = panel::prompt::builtin_templates()
        .into_iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| format!("no such template: {template_id}"))?;

    let (utterances, notes, config) = {
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        let utterances =
            repo::meeting_utterances(db.connection(), &meeting_id).map_err(|e| e.to_string())?;
        let notes = repo::meeting_notes(db.connection(), &meeting_id).map_err(|e| e.to_string())?;
        let config = state.provider.lock().map_err(|_| "lock poisoned")?.clone();
        (utterances, notes, config)
    };

    let generated = panel::generate(
        &state.llm,
        &config,
        state.keys.as_ref(),
        &template,
        &utterances,
        &notes,
    )
    .await
    .map_err(|e| e.to_string())?;

    let id = format!("p{}", now_ms());
    let content_json = serde_json::to_string(&generated.content).map_err(|e| e.to_string())?;

    {
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        repo::insert_panel(
            db.connection(),
            &id,
            &meeting_id,
            &template.id,
            &content_json,
            &generated.content.plaintext(),
            &generated.provider,
            &generated.model,
            now_ms(),
        )
        .map_err(|e| e.to_string())?;
    }

    if generated.report.had_hallucinations() {
        eprintln!(
            "panel {id}: dropped {} invented utterance citations and {} note citations",
            generated.report.dropped_utterances, generated.report.dropped_notes
        );
    }

    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::meeting_panels(db.connection(), &meeting_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "panel vanished after insert".to_string())
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
        .plugin(
            // Start/stop without leaving whatever you are in. The shortcut is
            // deliberately awkward: this fires globally, so it must not collide
            // with anything an app in the foreground might want.
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    // Key-down only. Without this the toggle fires twice per
                    // press and lands back where it started.
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if shortcut == &record_shortcut() {
                        toggle_recording(app);
                    }
                })
                .build(),
        )
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
            let builtins = panel::prompt::builtin_templates();
            let rows: Vec<(&str, &str, &str)> = builtins
                .iter()
                .map(|t| (t.id.as_str(), t.name.as_str(), t.prompt.as_str()))
                .collect();
            if let Err(err) = repo::ensure_builtin_templates(db.connection(), &rows, now_ms()) {
                eprintln!("could not seed templates: {err}");
            }

            app.manage(AppState {
                db: Mutex::new(db),
                db_path: path.to_string_lossy().into_owned(),
                sidecar: Mutex::new(None),
                meeting: Mutex::new(MeetingState::default()),
                provider: Mutex::new({
                    let mut config = ProviderConfig::preset(ProviderKind::Ollama);
                    // Dev override: point at whatever model is actually pulled
                    // locally without editing the preset.
                    if let Ok(model) = std::env::var("OATMEAL_PROVIDER_MODEL") {
                        if !model.trim().is_empty() {
                            config.model = model;
                        }
                    }
                    config
                }),
                keys: Box::new(Keychain),
                llm: LlmClient::new(),
                runtime: llm::bundled::Runtime::new(&dir),
                http: reqwest::Client::builder()
                    // No overall timeout: a 5 GB model on a slow line legitimately
                    // takes an hour, and a deadline here would look like a network
                    // fault. Stalls are caught by the read timeout instead.
                    .read_timeout(std::time::Duration::from_secs(60))
                    .build()
                    .unwrap_or_default(),
                cancel_download: Arc::new(AtomicBool::new(false)),
                recording_since: Mutex::new(None),
                candidates: Mutex::new(detect::Queue::new()),
                offered_events: Mutex::new(Vec::new()),
                pending_question: Mutex::new(None),
                last_permissions: Mutex::new(None),
                link_params: Mutex::new(link::LinkParams::default()),
            });

            // The menu bar item. Reported rather than fatal — the app is still
            // fully usable from its window without it.
            if let Err(err) = tray::install(app.handle()) {
                eprintln!("could not install the menu bar item: {err}");
            }
            refresh_tray(app.handle());

            // The watchers live in the sidecar, so detection cannot work unless
            // it is running. Started here rather than left to the Sidecar card,
            // which is a diagnostic surface the user has no reason to visit.
            {
                let settings =
                    load_settings(app.state::<AppState>().db.lock().unwrap().connection());
                if settings.mic_enabled || settings.calendar_enabled {
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        if let Err(err) = spawn_sidecar(&handle, false) {
                            eprintln!("detection needs the sidecar, which would not start: {err}");
                        }
                    });
                }
            }

            // Expires unanswered offers. Sharing the tray's timer would couple
            // two unrelated cadences; this one only has to be roughly right.
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    let state = handle.state::<AppState>();
                    let expired = state
                        .candidates
                        .lock()
                        .map(|mut q| q.expire(now_ms()))
                        .unwrap_or_default();
                    if expired.is_empty() {
                        continue;
                    }
                    let remaining = state
                        .candidates
                        .lock()
                        .map(|q| q.pending().to_vec())
                        .unwrap_or_default();
                    let _ = handle.emit("detect://candidates", &remaining);
                    if remaining.is_empty() {
                        hide_popup(&handle);
                    }
                });
            }

            // Ticks the menu bar clock. A timer rather than an event stream
            // because the elapsed time changes every second on its own, with
            // nothing happening in the app to hang an event off.
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    let recording = handle
                        .state::<AppState>()
                        .meeting
                        .lock()
                        .map(|m| m.is_recording())
                        .unwrap_or(false);
                    if recording {
                        // Only the title moves each second. Rebuilding the whole
                        // menu at 1 Hz would fight with a menu the user has open.
                        let elapsed = handle
                            .state::<AppState>()
                            .recording_since
                            .lock()
                            .ok()
                            .and_then(|since| *since)
                            .map(|started| now_ms() - started)
                            .unwrap_or(0);
                        if let Some(icon) =
                            handle.tray_by_id(&tauri::tray::TrayIconId::new("oatmeal"))
                        {
                            let _ = icon.set_title(Some(tray::format_elapsed(elapsed)));
                        }
                    }
                });
            }

            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                // Reported, not fatal: another app may already own the
                // combination, and losing a shortcut is not losing the app.
                if let Err(err) = app.global_shortcut().register(record_shortcut()) {
                    eprintln!("could not register the record shortcut: {err}");
                }
            }

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
            meeting_delete,
            providers_list,
            provider_current,
            provider_select,
            provider_set_key,
            provider_test,
            templates_list,
            panels_list,
            panel_delete,
            panel_generate,
            runtime_state,
            runtime_models,
            runtime_start,
            runtime_stop,
            runtime_model_status,
            runtime_install_server,
            runtime_install_model,
            runtime_cancel_download,
            detection_respond,
            detection_answer_app,
            detection_candidates,
            detection_pending_question,
            detection_settings,
            detection_set_settings,
            detection_rules_list,
            detection_rule_clear,
            detection_builtin_apps,
            folders_list,
            folder_create,
            folder_rename,
            folder_delete,
            folder_meetings,
            meeting_set_folder,
            search_transcripts,
            chat_ask,
            meeting_index,
            meeting_links,
            link_params_get,
            link_params_set
        ])
        .run(app_context())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_shortcut_keeps_all_three_modifiers() {
        // This is registered system-wide, so it takes the combination away from
        // every other app. Dropping a modifier to make it comfier would shadow
        // something the user cares about more than this.
        use tauri_plugin_global_shortcut::{Code, Modifiers};
        let shortcut = record_shortcut();
        assert_eq!(shortcut.key, Code::KeyR);
        assert!(shortcut.mods.contains(Modifiers::CONTROL));
        assert!(shortcut.mods.contains(Modifiers::ALT));
        assert!(shortcut.mods.contains(Modifiers::SUPER));
    }

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
