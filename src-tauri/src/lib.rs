pub mod chat;
pub mod db;
pub mod detect;
pub mod diag;
pub mod embed;
pub mod gcal;
pub mod link;
pub mod llm;
pub mod meeting;
pub mod notion;
pub mod panel;
pub mod retention;
pub mod search;
pub mod sidecar;
pub mod tray;
pub mod update;

use std::collections::{HashMap, HashSet};
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
    /// Google Calendar over OAuth, for calendars macOS does not sync.
    ///
    /// Additive: EventKit stays the default, and turning this off leaves
    /// detection exactly as it was.
    pub gcal: gcal::Connection,
    /// The last calendar window the sidecar reported, and the calendars it came
    /// from. Held so the menu bar and the settings list can read them without a
    /// round trip to EventKit that would need calendar permission again.
    pub upcoming: Mutex<Vec<detect::CalendarEvent>>,
    pub calendars: Mutex<Vec<detect::calendar::CalendarSource>>,
    /// What the sidecar last said about calendar access, and when it said it.
    /// The app was already being told this and throwing it away, which is why
    /// an empty list could only be explained by guessing.
    pub calendar_access: Mutex<Option<CalendarAccess>>,
    /// The calendars in the connected Google account, and its address. Cached
    /// because the settings screen reads them on every visit and the account
    /// changes about as often as the user connects one.
    pub google_calendars: Mutex<Vec<gcal::calendars::GoogleCalendar>>,
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
    // The user's configured window (G27). "Forever" stores no expiry, which the
    // sweeper reads as "keep".
    let expires = retention::Retention::expires_at(load_retention(db.connection()), now_ms());
    if let Err(err) = repo::finish_meeting(
        db.connection(),
        &meeting_id,
        now_ms(),
        audio_path.as_deref(),
        expires,
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

                // Auto-export, when the user asked for it. After indexing so
                // the page carries the finished article, and best-effort: a
                // Notion outage must not look like a failed recording.
                let auto = handle
                    .state::<AppState>()
                    .db
                    .lock()
                    .ok()
                    .and_then(|db| {
                        repo::get_setting(db.connection(), SETTING_NOTION_AUTO)
                            .ok()
                            .flatten()
                    })
                    .map(|v| v == "1")
                    .unwrap_or(false);
                if auto {
                    match notion_export(handle.clone(), id.clone()).await {
                        Ok(result) => eprintln!(
                            "exported {id} to Notion page {} ({})",
                            result.page_id,
                            if result.created { "created" } else { "updated" }
                        ),
                        Err(err) => eprintln!("auto-export of {id} failed: {err}"),
                    }
                }
            }
            Err(err) => eprintln!("failed to index {id}: {err}"),
        }
    });
}

/// Creates a meeting row and tells the sidecar to start recording into it.
/// A meeting with nothing recorded in it yet.
///
/// Deliberately not `meeting_start`. That one transitions the lifecycle, tells
/// the sidecar to capture, and fails outright when the sidecar is not running —
/// all correct for pressing record, and all wrong for pressing `+`. Someone
/// opening a note to type into is not starting a recording, and should not need
/// a working microphone to do it.
#[tauri::command]
fn meeting_create(
    state: tauri::State<'_, AppState>,
    title: Option<String>,
) -> Result<String, String> {
    let started = now_ms();
    let id = format!("m{started}");
    let title = title.unwrap_or_else(|| "New note".to_string());

    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::insert_empty_meeting(db.connection(), &id, &title, started).map_err(|e| e.to_string())?;
    Ok(id)
}

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

// ------------------------------------------------------------- google calendar

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GcalSettings {
    /// Whether a refresh token is stored. Not whether it still works — that is
    /// only knowable by using it.
    pub connected: bool,
    pub client_id: Option<String>,
    /// Whether a client secret is stored. Never the secret itself.
    pub has_client_secret: bool,
    pub enabled: bool,
}

fn gcal_client_id(conn: &rusqlite::Connection) -> Option<String> {
    repo::get_setting(conn, SETTING_GCAL_CLIENT_ID)
        .ok()
        .flatten()
        .filter(|id| !id.trim().is_empty())
}

#[tauri::command]
fn gcal_settings(state: tauri::State<'_, AppState>) -> Result<GcalSettings, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    let conn = db.connection();
    Ok(GcalSettings {
        connected: state.gcal.is_connected(state.keys.as_ref()),
        client_id: gcal_client_id(conn),
        // Whether one is stored, never the value. The card needs to show
        // "set"; it has no business reading it back out of the Keychain.
        has_client_secret: state.keys.has(gcal::connection::CLIENT_SECRET_KEY),
        enabled: repo::get_setting(conn, SETTING_GCAL_ENABLED)
            .ok()
            .flatten()
            .map(|v| v == "1")
            .unwrap_or(false),
    })
}

#[tauri::command]
fn gcal_set_client_id(state: tauri::State<'_, AppState>, client_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::set_setting(db.connection(), SETTING_GCAL_CLIENT_ID, client_id.trim())
        .map_err(|e| e.to_string())
}

/// Stores the OAuth client secret. Never returned, logged, or echoed back.
///
/// An empty string clears it rather than storing a blank — `client_secret=` is
/// not "no secret" to Google, it is a wrong one.
#[tauri::command]
fn gcal_set_client_secret(
    state: tauri::State<'_, AppState>,
    client_secret: String,
) -> Result<(), String> {
    let secret = client_secret.trim();
    if secret.is_empty() {
        return state
            .keys
            .delete(gcal::connection::CLIENT_SECRET_KEY)
            .map_err(|e| e.to_string());
    }
    state
        .keys
        .set(gcal::connection::CLIENT_SECRET_KEY, secret)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn gcal_set_enabled(state: tauri::State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::set_setting(
        db.connection(),
        SETTING_GCAL_ENABLED,
        if enabled { "1" } else { "0" },
    )
    .map_err(|e| e.to_string())
}

/// Runs the whole OAuth flow: opens the browser, catches the redirect,
/// redeems the code.
///
/// On a blocking thread — the loopback listener is a plain `TcpListener`, and
/// the flow is one linear sequence that reads far worse split across an async
/// boundary for no gain.
#[tauri::command]
async fn gcal_connect(app: tauri::AppHandle) -> Result<gcal::connection::FlowOutcome, String> {
    let (client_id, client_secret, http) = {
        let state = app.state::<AppState>();
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        let client_id = gcal_client_id(db.connection()).ok_or(
            "no Google OAuth client id is set — create a Desktop app client in Google \
             Cloud Console and paste its id",
        )?;
        let client_secret = state
            .keys
            .get(gcal::connection::CLIENT_SECRET_KEY)
            .map_err(|e| e.to_string())?;
        (client_id, client_secret, state.http.clone())
    };

    let for_exchange = client_id.clone();
    let tokens = tauri::async_runtime::spawn_blocking(move || {
        gcal::connection::authorize(
            &client_id,
            |url| {
                tauri_plugin_opener::open_url(url, None::<&str>)
                    .map_err(|e| format!("could not open the browser: {e}"))
            },
            |code, verifier, redirect_uri| {
                tauri::async_runtime::block_on(gcal::token::exchange(
                    &http,
                    &for_exchange,
                    client_secret.as_deref(),
                    code,
                    verifier,
                    redirect_uri,
                    now_ms(),
                ))
            },
        )
    })
    .await
    .map_err(|e| format!("the authorization thread failed: {e}"))?;

    match tokens {
        Ok(tokens) => {
            let state = app.state::<AppState>();
            state
                .gcal
                .adopt(state.keys.as_ref(), tokens)
                .map_err(|e| e.to_string())?;
            Ok(gcal::connection::FlowOutcome {
                connected: true,
                reason: None,
            })
        }
        Err(err) => Ok(gcal::connection::FlowOutcome {
            connected: false,
            reason: Some(err.to_string()),
        }),
    }
}

/// Upcoming Google events, or an explanation.
///
/// Returns early when the source is switched off so a disconnected user pays
/// nothing for the poll.
async fn gcal_upcoming(app: &tauri::AppHandle) -> Result<Vec<detect::CalendarEvent>, String> {
    let (client_id, enabled, http) = {
        let state = app.state::<AppState>();
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        let conn = db.connection();
        let enabled = repo::get_setting(conn, SETTING_GCAL_ENABLED)
            .ok()
            .flatten()
            .map(|v| v == "1")
            .unwrap_or(false);
        (gcal_client_id(conn), enabled, state.http.clone())
    };

    if !enabled {
        return Ok(Vec::new());
    }
    let client_id = client_id.ok_or("not connected: no client id")?;

    let state = app.state::<AppState>();

    // Only the calendars the user left switched on. Reading a calendar they
    // hid would put its meetings in the popup anyway, which makes the switch
    // a lie; reading only `primary` — what this used to do — misses every
    // meeting on a shared or team calendar.
    let wanted: Vec<String> = {
        let hidden = {
            let db = state.db.lock().map_err(|_| "db lock poisoned")?;
            hidden_calendars(db.connection())
        };
        state
            .google_calendars
            .lock()
            .map_err(|_| "google calendars lock poisoned")?
            .iter()
            .filter(|c| !hidden.contains(&gcal::calendars::source_id(&c.id)))
            .map(|c| c.id.clone())
            .collect()
    };
    // Nothing cached yet — the account has not been read since launch. The
    // primary calendar is the honest fallback, not "no meetings at all".
    let wanted = if wanted.is_empty() {
        vec!["primary".to_string()]
    } else {
        wanted
    };

    state
        .gcal
        .upcoming(
            &http,
            state.keys.as_ref(),
            &client_id,
            &wanted,
            now_ms(),
            24 * 60 * 60 * 1000,
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn gcal_disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .gcal
        .disconnect(state.keys.as_ref())
        .map_err(|e| e.to_string())
}

// --------------------------------------------------------------------- notion

/// Keychain reference for the Notion integration token.
///
/// The same store as the LLM keys: a token that can read and write a user's
/// workspace has no business in SQLite.
pub const NOTION_KEY: &str = "notion";
pub const SETTING_NOTION_DATABASE: &str = "notion.database_id";
pub const SETTING_NOTION_TRANSCRIPT: &str = "notion.include_transcript";
pub const SETTING_NOTION_AUTO: &str = "notion.auto_export";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionSettings {
    /// Never the token itself.
    pub has_token: bool,
    pub database_id: Option<String>,
    pub include_transcript: bool,
    pub auto_export: bool,
}

fn notion_client(state: &AppState) -> Result<notion::Notion, String> {
    let token = state
        .keys
        .get(NOTION_KEY)
        .map_err(|e| e.to_string())?
        .ok_or("no Notion token is stored")?;
    Ok(notion::Notion::new(token))
}

#[tauri::command]
fn notion_settings(state: tauri::State<'_, AppState>) -> Result<NotionSettings, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    let conn = db.connection();
    Ok(NotionSettings {
        has_token: state.keys.has(NOTION_KEY),
        database_id: repo::get_setting(conn, SETTING_NOTION_DATABASE)
            .ok()
            .flatten(),
        include_transcript: repo::get_setting(conn, SETTING_NOTION_TRANSCRIPT)
            .ok()
            .flatten()
            .map(|v| v == "1")
            .unwrap_or(false),
        auto_export: repo::get_setting(conn, SETTING_NOTION_AUTO)
            .ok()
            .flatten()
            .map(|v| v == "1")
            .unwrap_or(false),
    })
}

/// Stores the integration token, or clears it when given an empty string.
#[tauri::command]
fn notion_set_token(state: tauri::State<'_, AppState>, token: String) -> Result<(), String> {
    if token.trim().is_empty() {
        return state.keys.delete(NOTION_KEY).map_err(|e| e.to_string());
    }
    state
        .keys
        .set(NOTION_KEY, token.trim())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn notion_set_options(
    state: tauri::State<'_, AppState>,
    database_id: Option<String>,
    include_transcript: bool,
    auto_export: bool,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    let conn = db.connection();
    if let Some(id) = database_id {
        let _ = repo::set_setting(conn, SETTING_NOTION_DATABASE, &id);
    }
    let _ = repo::set_setting(
        conn,
        SETTING_NOTION_TRANSCRIPT,
        if include_transcript { "1" } else { "0" },
    );
    let _ = repo::set_setting(
        conn,
        SETTING_NOTION_AUTO,
        if auto_export { "1" } else { "0" },
    );
    Ok(())
}

/// Databases the integration can see.
#[tauri::command]
async fn notion_databases(app: tauri::AppHandle) -> Result<Vec<notion::client::Database>, String> {
    let client = {
        let state = app.state::<AppState>();
        notion_client(&state)?
    };
    client.databases().await.map_err(|e| e.to_string())
}

/// Exports a meeting, creating its page or updating the one it already has.
#[tauri::command]
async fn notion_export(
    app: tauri::AppHandle,
    meeting_id: String,
) -> Result<notion::export::ExportResult, String> {
    // Everything the database is needed for happens up front: `Connection` is
    // `!Send`, so its guard cannot be held across the network calls below.
    let (client, database_id, input, existing) = {
        let state = app.state::<AppState>();
        let client = notion_client(&state)?;
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        let conn = db.connection();

        let database_id = repo::get_setting(conn, SETTING_NOTION_DATABASE)
            .ok()
            .flatten()
            .ok_or("no Notion database has been chosen")?;
        let include_transcript = repo::get_setting(conn, SETTING_NOTION_TRANSCRIPT)
            .ok()
            .flatten()
            .map(|v| v == "1")
            .unwrap_or(false);

        let input = notion::export::gather(conn, &meeting_id, include_transcript)
            .map_err(|e| e.to_string())?;
        let existing = notion::export::record_for(conn, &meeting_id).map_err(|e| e.to_string())?;
        (client, database_id, input, existing)
    };

    let database = client
        .databases()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|d| d.id == database_id)
        .ok_or("the chosen Notion database is no longer shared with the integration")?;

    let result = notion::export::export(&client, &database, &input, existing.as_ref())
        .await
        .map_err(|e| e.to_string())?;

    let state = app.state::<AppState>();
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    notion::export::save(db.connection(), &input, &database.id, &result, now_ms())
        .map_err(|e| e.to_string())?;

    Ok(result)
}

// -------------------------------------------------------------------- privacy

/// What the privacy panel shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySnapshot {
    pub retention: retention::Retention,
    /// Audio files currently on disk.
    pub audio_files: i64,
    pub audio_bytes: u64,
    /// Every generation and where it went.
    pub generations: Vec<repo::Provenance>,
    /// Always false. Asserted rather than claimed — see `no_telemetry`.
    pub telemetry: bool,
}

pub fn load_retention(conn: &rusqlite::Connection) -> retention::Retention {
    repo::get_setting(conn, SETTING_RETENTION)
        .ok()
        .flatten()
        .and_then(|value| retention::Retention::parse(&value))
        .unwrap_or_default()
}

#[tauri::command]
fn privacy_snapshot(state: tauri::State<'_, AppState>) -> Result<PrivacySnapshot, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    let conn = db.connection();
    let (audio_files, audio_bytes) = repo::audio_footprint(conn).map_err(|e| e.to_string())?;

    Ok(PrivacySnapshot {
        retention: load_retention(conn),
        audio_files,
        audio_bytes,
        generations: repo::panel_provenance(conn, 50).map_err(|e| e.to_string())?,
        telemetry: false,
    })
}

#[tauri::command]
fn privacy_set_retention(
    state: tauri::State<'_, AppState>,
    retention: retention::Retention,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::set_setting(db.connection(), SETTING_RETENTION, &retention.as_str())
        .map_err(|e| e.to_string())
}

/// Deletes every audio file. Transcripts survive.
#[tauri::command]
fn privacy_purge_audio(
    state: tauri::State<'_, AppState>,
) -> Result<retention::SweepReport, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    retention::purge_all(db.connection()).map_err(|e| e.to_string())
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
/// How long audio is kept. `"forever"` or a day count.
pub const SETTING_RETENTION: &str = "privacy.audio_retention";
/// The user's own Google OAuth client id. Not a secret, so it lives in settings
/// rather than the Keychain. Its other half does not — see
/// `gcal::connection::CLIENT_SECRET_KEY`.
pub const SETTING_GCAL_CLIENT_ID: &str = "gcal.client_id";
/// Whether the Google calendar source is switched on.
pub const SETTING_GCAL_ENABLED: &str = "gcal.enabled";
/// Calendars the user has switched off, as a JSON array of EventKit ids.
pub const SETTING_HIDDEN_CALENDARS: &str = "calendar.hidden";
/// Offer entries with nobody else on them.
pub const SETTING_INCLUDE_SOLO: &str = "detect.include_solo_events";
/// Show the next meeting in the macOS menu bar.
pub const SETTING_SHOW_UPCOMING: &str = "tray.show_upcoming";

/// Detection settings as the UI sees them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionSettings {
    /// How long before a calendar event to offer. Default 90s, per SPEC.
    pub lead_ms: i64,
    pub mic_enabled: bool,
    pub calendar_enabled: bool,
    /// Offer entries with no link, no location and nobody else on them.
    /// Off by default: those are focus time, and offering to record one is
    /// what teaches a user to dismiss the popup on sight.
    pub include_solo_events: bool,
    /// Put the next meeting in the macOS menu bar.
    pub show_upcoming_in_menu_bar: bool,
}

impl Default for DetectionSettings {
    fn default() -> Self {
        Self {
            lead_ms: 90_000,
            include_solo_events: false,
            show_upcoming_in_menu_bar: false,
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
        include_solo_events: read(SETTING_INCLUDE_SOLO)
            .map(|v| v == "1")
            .unwrap_or(defaults.include_solo_events),
        show_upcoming_in_menu_bar: read(SETTING_SHOW_UPCOMING)
            .map(|v| v == "1")
            .unwrap_or(defaults.show_upcoming_in_menu_bar),
    }
}

/// The calendars whose events are ignored.
///
/// Stored as the *hidden* set rather than the visible one so a calendar added
/// after the choice was made shows up by default. The other way round, every
/// new calendar would arrive silently switched off.
pub fn hidden_calendars(conn: &rusqlite::Connection) -> HashSet<String> {
    repo::get_setting(conn, SETTING_HIDDEN_CALENDARS)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
        .into_iter()
        .collect()
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
                    join_url: None,
                    at_ms: now_ms(),
                };
                offer_candidate(app, candidate);
            }
        }
    }
}

// ------------------------------------------------------- visible calendars

/// Every calendar EventKit reports, with the user's on/off choice applied.
///
/// Empty until the sidecar has reported once — that is honestly "not known
/// yet" rather than "you have no calendars", and the UI says so.
#[tauri::command]
fn calendar_sources(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<detect::calendar::CalendarSource>, String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    let hidden = hidden_calendars(db.connection());
    drop(db);
    let known = state
        .calendars
        .lock()
        .map_err(|_| "calendars lock poisoned")?;

    // Every calendar in the connected account, as its own row. Read from the
    // cache so the screen renders at once; `calendar_refresh_google` is what
    // fills it, behind the render.
    let google = state
        .google_calendars
        .lock()
        .map_err(|_| "google calendars lock poisoned")?
        .clone();
    Ok(detect::calendar::sources_for_display(
        &known, &hidden, &google,
    ))
}

/// Fetches the account's calendars and caches them, returning its address.
///
/// Separate from `calendar_sources` so the settings screen renders from cache
/// at once and the network happens behind it — and so a failure to reach
/// Google leaves the last known list on screen rather than emptying it.
#[tauri::command]
async fn calendar_refresh_google(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (client_id, http) = {
        let state = app.state::<AppState>();
        if !state.gcal.is_connected(state.keys.as_ref()) {
            return Ok(None);
        }
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        let Some(client_id) = gcal_client_id(db.connection()) else {
            return Ok(None);
        };
        (client_id, state.http.clone())
    };

    let calendars = {
        let state = app.state::<AppState>();
        state
            .gcal
            .calendars(&http, state.keys.as_ref(), &client_id, now_ms())
            .await
            .map_err(|e| e.to_string())?
    };

    let email = gcal::calendars::account_email(&calendars);
    let state = app.state::<AppState>();
    *state
        .google_calendars
        .lock()
        .map_err(|_| "google calendars lock poisoned")? = calendars;
    Ok(email)
}

/// Switches one calendar on or off.
#[tauri::command]
fn calendar_set_visible(
    state: tauri::State<'_, AppState>,
    calendar_id: String,
    visible: bool,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;

    let mut hidden = hidden_calendars(db.connection());
    if visible {
        hidden.remove(&calendar_id);
    } else {
        hidden.insert(calendar_id);
    }
    // Sorted so the stored value is stable rather than reordering on every
    // write — a set has no order, and a diff of the settings row should mean
    // something changed.
    let mut list: Vec<String> = hidden.into_iter().collect();
    list.sort();
    repo::set_setting(
        db.connection(),
        SETTING_HIDDEN_CALENDARS,
        &serde_json::to_string(&list).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Switches every calendar back on.
#[tauri::command]
fn calendar_reset_visibility(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::set_setting(db.connection(), SETTING_HIDDEN_CALENDARS, "[]").map_err(|e| e.to_string())
}

/// Sets one of the two calendar display switches.
#[tauri::command]
fn calendar_set_display(
    state: tauri::State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<(), String> {
    let setting = match key.as_str() {
        "includeSoloEvents" => SETTING_INCLUDE_SOLO,
        "showUpcomingInMenuBar" => SETTING_SHOW_UPCOMING,
        // Named rather than ignored: a typo here would otherwise be a switch
        // that moves and does nothing.
        other => return Err(format!("unknown display setting '{other}'")),
    };
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::set_setting(db.connection(), setting, if enabled { "1" } else { "0" })
        .map_err(|e| e.to_string())
}

/// What the sidecar last reported about calendar access.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarAccess {
    pub authorized: bool,
    pub checked_at_ms: i64,
}

/// The diagnosis for an empty calendar list, from the app rather than a guess.
///
/// `None` means the sidecar has never reported, which is itself the answer —
/// it is not running, or the calendar watcher was never switched on.
#[tauri::command]
fn calendar_access(state: tauri::State<'_, AppState>) -> Result<Option<CalendarAccess>, String> {
    Ok(*state
        .calendar_access
        .lock()
        .map_err(|_| "calendar access lock poisoned")?)
}

/// The last lines the sidecar wrote, oldest first.
#[tauri::command]
fn sidecar_log_tail(app: tauri::AppHandle, limit: usize) -> Result<Vec<String>, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let path = diag::log_path(&dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    diag::tail(&path, limit.clamp(1, 2000)).map_err(|e| e.to_string())
}

/// Where the log is, so it can be opened in Finder or attached to a report.
#[tauri::command]
fn sidecar_log_path(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(diag::log_path(&dir).to_string_lossy().to_string())
}

/// Writes one supervisor event to the sidecar log.
///
/// Everything, not just errors: "the watcher started" is exactly as useful as
/// "the watcher failed" when the question is why a list is empty.
fn log_supervisor_event(app: &tauri::AppHandle, event: &sidecar::SupervisorEvent) {
    let line = match event {
        sidecar::SupervisorEvent::Stderr { line } => line.clone(),
        sidecar::SupervisorEvent::Spawned { pid, attempt } => {
            format!("[supervisor] spawned pid={pid} attempt={attempt}")
        }
        sidecar::SupervisorEvent::Exited {
            code,
            restarting_in_ms,
        } => format!("[supervisor] exited code={code:?} restarting_in_ms={restarting_in_ms:?}"),
        sidecar::SupervisorEvent::GaveUp { reason } => format!("[supervisor] gave up: {reason}"),
        sidecar::SupervisorEvent::Garbled { line, error } => {
            format!("[supervisor] unparsed line ({error}): {line}")
        }
        // The parsed stream is already the app's own state; logging every
        // transcript partial would bury the diagnostics in the transcript.
        sidecar::SupervisorEvent::Event { event } => match event {
            sidecar::SidecarEvent::CalendarEvents {
                events,
                calendars,
                authorized,
            } => format!(
                "[calendar] authorized={authorized} calendars={} events={}",
                calendars.len(),
                events.len()
            ),
            sidecar::SidecarEvent::Ready { version, .. } => format!("[sidecar] ready {version}"),
            sidecar::SidecarEvent::Error { message, fatal } => {
                format!("[sidecar] error (fatal={fatal}): {message}")
            }
            _ => return,
        },
    };
    if let Ok(dir) = app.path().app_data_dir() {
        let stamped = format!("{} {line}", now_ms());
        let _ = diag::append(&diag::log_path(&dir), &stamped);
    }
}

/// The calendar reported its upcoming window.
fn on_calendar_events(app: &tauri::AppHandle, events: &[detect::CalendarEvent]) {
    let state = app.state::<AppState>();
    if let Ok(mut window) = state.upcoming.lock() {
        // Kept whatever happens below: the menu bar shows what is coming up
        // even while a meeting is being recorded, and even when detection is
        // switched off entirely.
        *window = events.to_vec();
    }
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
    let hidden = hidden_calendars(db.connection());
    drop(db);
    if !settings.calendar_enabled {
        return;
    }

    // Hidden calendars are dropped before anything else looks at them, so a
    // switched-off calendar cannot raise a popup by any route.
    let events: Vec<detect::CalendarEvent> = detect::calendar::visible(events, &hidden)
        .into_iter()
        .cloned()
        .collect();

    let already = state
        .offered_events
        .lock()
        .map(|o| o.clone())
        .unwrap_or_default();
    let due = detect::calendar::due(
        &events,
        now_ms(),
        settings.lead_ms,
        &already,
        settings.include_solo_events,
    );

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
                // The link the entry carried, if any. `conferencing_url` also
                // digs it out of the description, which is where most calendar
                // integrations paste it.
                join_url: detect::calendar::conferencing_url(event)
                    .filter(|url| url.starts_with("http")),
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

/// How big the offer is. A pill, not a dialog.
pub const POPUP_SIZE: (f64, f64) = (460.0, 72.0);

/// How far below the top of the screen it sits.
///
/// Clear of the menu bar rather than under it: an offer hidden behind the
/// clock is an offer nobody answers.
pub const POPUP_TOP_MARGIN: f64 = 8.0;

/// Where the offer should appear on a screen of a given size.
///
/// Top centre, where macOS itself puts transient notifications and where
/// Notion puts the same offer. The previous window took whatever position
/// macOS handed it, which put a 190px dialog in the middle of whatever the
/// user was reading.
pub fn popup_origin(screen_width: f64, menu_bar_height: f64) -> (f64, f64) {
    let x = ((screen_width - POPUP_SIZE.0) / 2.0).max(0.0);
    (x, menu_bar_height + POPUP_TOP_MARGIN)
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
        .inner_size(POPUP_SIZE.0, POPUP_SIZE.1)
        .resizable(false)
        .always_on_top(true)
        // No traffic lights: this is a transient offer, and a close button
        // invites dismissing it in a way that records no decision.
        .decorations(false)
        // So the pill can have rounded corners instead of sitting in a
        // rectangle of window. Needs `macos-private-api`, which rules out the
        // Mac App Store — Oatmeal ships Developer ID direct, so that costs
        // nothing here.
        .transparent(true)
        .skip_taskbar(true)
        .focused(false)
        // Visible on whichever Space the user is on, which for a video call is
        // rarely the one Oatmeal was launched in.
        .visible_on_all_workspaces(true)
        .build();

    match built {
        Ok(window) => {
            // Placed explicitly. Left to macOS it lands wherever the last
            // window did, which is the middle of whatever is being read.
            if let Ok(Some(monitor)) = window.primary_monitor() {
                let scale = monitor.scale_factor();
                let size = monitor.size().to_logical::<f64>(scale);
                let position = monitor.position().to_logical::<f64>(scale);
                let (x, y) = popup_origin(size.width, 0.0);
                let _ = window
                    .set_position(tauri::LogicalPosition::new(position.x + x, position.y + y));
            }
        }
        Err(err) => eprintln!("could not show the detection popup: {err}"),
    }
}

/// Opens the meeting link and starts recording, in that order.
///
/// One action because it is one intention. Joining and recording as two
/// separate steps means doing the second one late, from another window, after
/// the call has already started.
#[tauri::command]
async fn detection_join(
    app: tauri::AppHandle,
    candidate_id: String,
    url: Option<String>,
) -> Result<Option<String>, String> {
    if let Some(url) = url.filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
        // Reported rather than fatal: failing to open the browser is no reason
        // to skip the recording the user just asked for.
        if let Err(err) = tauri_plugin_opener::open_url(&url, None::<&str>) {
            eprintln!("could not open the meeting link: {err}");
        }
    }
    detection_respond(app, candidate_id, detect::Outcome::Start)
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

/// The next meeting for the menu bar, or nothing.
///
/// Reads the window the sidecar last reported rather than asking EventKit:
/// this runs on the tray ticker, and hitting the calendar store at 1 Hz to
/// render a label would be absurd.
fn next_up_title(app: &tauri::AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    let (show, hidden, include_solo) = {
        let db = state.db.lock().ok()?;
        let settings = load_settings(db.connection());
        (
            settings.show_upcoming_in_menu_bar,
            hidden_calendars(db.connection()),
            settings.include_solo_events,
        )
    };
    if !show {
        return None;
    }

    let events = state.upcoming.lock().ok()?.clone();
    let now = now_ms();
    // Same two filters detection uses. A calendar switched off in settings must
    // not reappear in the menu bar, and neither should a solo hold when the
    // user has said those are not meetings.
    let mut soonest: Vec<&detect::CalendarEvent> = detect::calendar::visible(&events, &hidden)
        .into_iter()
        .filter(|event| include_solo || detect::calendar::is_meeting_shaped(event))
        .filter(|event| event.starts_at >= now)
        .collect();
    soonest.sort_by_key(|event| event.starts_at);

    let next = soonest.first()?;
    tray::next_up_label(
        next.title.as_deref().unwrap_or_default(),
        next.starts_at,
        now,
    )
}

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
    let next_up = next_up_title(app);
    let _ = icon.set_title(Some(tray::tray_title(
        recording,
        elapsed,
        next_up.as_deref(),
    )));
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
                sidecar::SidecarEvent::CalendarEvents {
                    events,
                    calendars,
                    authorized,
                } => {
                    if let Ok(mut access) = app_handle.state::<AppState>().calendar_access.lock() {
                        *access = Some(CalendarAccess {
                            authorized: *authorized,
                            checked_at_ms: now_ms(),
                        });
                    }
                    if let Ok(mut known) = app_handle.state::<AppState>().calendars.lock() {
                        // Replaced wholesale rather than merged: a calendar the
                        // user deleted in Calendar.app should leave the list.
                        *known = calendars.clone();
                    }
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

        log_supervisor_event(&app_handle, &event);
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
            let _ = for_callback.send(&sidecar::SidecarCommand::Permissions {
                request: false,
                pane: None,
            });
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

/// Whether the chosen local model is actually installed.
///
/// Only Ollama can answer this — LM Studio has no registry to ask and the
/// bundled runtime has its own model picker — so everything else reports
/// installed rather than inventing a state it cannot check.
#[tauri::command]
async fn provider_model_available(
    app: tauri::AppHandle,
) -> Result<llm::ollama::ModelAvailability, String> {
    let (kind, base_url, model, http) = {
        let state = app.state::<AppState>();
        let config = state.provider.lock().map_err(|_| "lock poisoned")?.clone();
        (
            config.kind,
            config.base_url,
            config.model,
            state.http.clone(),
        )
    };
    if kind != llm::provider::ProviderKind::Ollama {
        return Ok(llm::ollama::ModelAvailability::Installed { model });
    }

    let response = http
        .get(llm::ollama::tags_url(&base_url))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
    let body = match response {
        // Not running is a different problem from not installed, and the user
        // fixes them in different places.
        Err(err) => {
            return Ok(llm::ollama::ModelAvailability::Unreachable {
                detail: format!("could not reach Ollama at {base_url}: {err}"),
            })
        }
        Ok(response) => response.text().await.map_err(|e| e.to_string())?,
    };
    Ok(llm::ollama::availability_from(&model, &body))
}

/// Pulls the chosen model. Progress arrives on `runtime://download`, the same
/// channel the bundled runtime uses, so the UI has one progress renderer.
#[tauri::command]
async fn provider_pull_model(app: tauri::AppHandle) -> Result<(), String> {
    use futures_util::StreamExt;

    let (base_url, model, http) = {
        let state = app.state::<AppState>();
        let config = state.provider.lock().map_err(|_| "lock poisoned")?.clone();
        (config.base_url, config.model, state.http.clone())
    };

    let response = http
        .post(llm::ollama::pull_url(&base_url))
        .json(&serde_json::json!({ "model": model, "stream": true }))
        .send()
        .await
        .map_err(|e| format!("could not reach Ollama at {base_url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Ollama refused the pull: {}", response.status()));
    }

    // Streamed line by line: a multi-gigabyte pull with no progress is
    // indistinguishable from a hang, and this is the one place the user is
    // asked to wait without anything to look at.
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = buffer.find('\n') {
            let line: String = buffer.drain(..=newline).collect();
            let Ok(parsed) = serde_json::from_str::<llm::ollama::PullLine>(line.trim()) else {
                continue;
            };
            if let Some(error) = parsed.error {
                return Err(error);
            }
            let _ = app.emit(
                "runtime://download",
                &llm::ollama::progress_from(&parsed, &model),
            );
        }
    }
    Ok(())
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

/// Asks GitHub whether anything newer exists.
///
/// Returns rather than throws when the check simply fails: a laptop on a train
/// should not show an error card, and an update check is never the reason to
/// interrupt someone. Genuine misconfiguration — an absent public key — is a
/// different matter and is reported.
#[tauri::command]
async fn update_check(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<update::UpdateStatus, String> {
    use tauri_plugin_updater::UpdaterExt;

    let current_version = app.package_info().version.to_string();
    let skipped = {
        let db = state.db.lock().map_err(|_| "db lock poisoned")?;
        repo::get_setting(db.connection(), update::SETTING_SKIPPED_VERSION)
            .map_err(|e| e.to_string())?
    };

    let found = app
        .updater()
        .map_err(|e| format!("the updater is not configured: {e}"))?
        .check()
        .await
        .map_err(|e| e.to_string())?;

    let (available_version, notes) = match &found {
        Some(update) => (Some(update.version.clone()), update.body.clone()),
        None => (None, None),
    };

    Ok(update::UpdateStatus {
        decision: update::decide(available_version.as_deref(), skipped.as_deref()),
        current_version,
        available_version,
        notes,
    })
}

/// Downloads the new bundle, swaps it in, and restarts.
///
/// Checks again rather than holding the handle from `update_check`: the handle
/// borrows the app and would have to be parked in shared state for what is a
/// single extra HTTP round trip on an action the user just clicked.
#[tauri::command]
async fn update_install(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    let update = app
        .updater()
        .map_err(|e| format!("the updater is not configured: {e}"))?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("there is nothing newer to install")?;

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;

    // Never returns.
    app.restart();
}

/// Remembers that the user does not want to hear about this exact version.
#[tauri::command]
fn update_skip(state: tauri::State<'_, AppState>, version: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db lock poisoned")?;
    repo::set_setting(db.connection(), update::SETTING_SKIPPED_VERSION, &version)
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        // Signed in-place updates. The updater refuses a manifest that is not
        // signed by the configured public key, so a release cannot be replaced
        // by anything the maintainer did not sign.
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            // Retention (G27): audio past its window is gone on next launch.
            // Before recovery, so a meeting interrupted long ago has its audio
            // swept in the same pass rather than surviving until the launch
            // after next.
            match retention::sweep(db.connection(), now_ms()) {
                Ok(report) if report.touched() > 0 => eprintln!(
                    "retention: removed {} audio file(s), {} already gone, {} KB freed",
                    report.deleted,
                    report.already_missing,
                    report.freed_bytes / 1024
                ),
                Ok(_) => {}
                Err(err) => eprintln!("retention sweep failed: {err}"),
            }

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
                gcal: gcal::Connection::new(),
                upcoming: Mutex::new(Vec::new()),
                calendars: Mutex::new(Vec::new()),
                calendar_access: Mutex::new(None),
                google_calendars: Mutex::new(Vec::new()),
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

            // Google Calendar, when connected and switched on. A separate poll
            // from EventKit's because it lives in Rust rather than the sidecar;
            // both feed the same queue, and the dedup there is what keeps one
            // meeting from producing two offers.
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(300));
                    let events = tauri::async_runtime::block_on(gcal_upcoming(&handle));
                    match events {
                        Ok(events) if !events.is_empty() => {
                            on_calendar_events(&handle, &events);
                        }
                        Ok(_) => {}
                        // Not shouted about on a loop: "not connected" is the
                        // ordinary state for most users.
                        Err(err) if err.contains("not connected") => {}
                        Err(err) => eprintln!("google calendar poll failed: {err}"),
                    }
                });
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
            provider_model_available,
            provider_pull_model,
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
            detection_join,
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
            privacy_snapshot,
            privacy_set_retention,
            privacy_purge_audio,
            notion_settings,
            notion_set_token,
            notion_set_options,
            notion_databases,
            notion_export,
            calendar_access,
            calendar_sources,
            calendar_refresh_google,
            sidecar_log_tail,
            sidecar_log_path,
            calendar_set_visible,
            calendar_reset_visibility,
            calendar_set_display,
            gcal_settings,
            gcal_set_client_id,
            gcal_set_client_secret,
            gcal_set_enabled,
            gcal_connect,
            gcal_disconnect,
            meeting_create,
            meeting_index,
            meeting_links,
            link_params_get,
            link_params_set,
            update_check,
            update_install,
            update_skip
        ])
        .run(app_context())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod popup_tests {
    use super::*;

    #[test]
    fn the_offer_sits_at_the_top_centre() {
        // Where macOS puts its own transient notifications, and where Notion
        // puts this exact offer. The window used to take whatever position
        // macOS handed it, which put a dialog in the middle of whatever was
        // being read.
        let (x, y) = popup_origin(1440.0, 0.0);
        assert_eq!(x, (1440.0 - POPUP_SIZE.0) / 2.0);
        assert_eq!(y, POPUP_TOP_MARGIN);
    }

    #[test]
    fn it_clears_the_menu_bar() {
        // Under the clock is where an offer goes to be ignored.
        let (_, y) = popup_origin(1440.0, 24.0);
        assert!(y > 24.0, "{y} would sit under the menu bar");
    }

    #[test]
    fn a_narrow_screen_does_not_push_it_off_the_left_edge() {
        // A negative origin puts half the pill past the edge of the display,
        // where the buttons cannot be clicked at all.
        let (x, _) = popup_origin(320.0, 0.0);
        assert_eq!(x, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The privacy panel claims no telemetry. This checks the claim against the
    /// source rather than trusting the sentence.
    ///
    /// Every outbound host in this app is one the user chose — their LLM
    /// provider, their embedding model, Hugging Face and GitHub when they ask
    /// for a download. Nothing reports back to us, and nothing should start to
    /// without this test failing first.
    #[test]
    fn the_source_contains_no_analytics_endpoints() {
        const FORBIDDEN: &[&str] = &[
            "google-analytics",
            "googletagmanager",
            "segment.io",
            "segment.com",
            "mixpanel",
            "amplitude.com",
            "posthog",
            "sentry.io",
            "bugsnag",
            "datadoghq",
            "plausible.io",
            "matomo",
            "hotjar",
            "fullstory",
            "logrocket",
            // Call shapes, not the bare word "telemetry" — the privacy panel
            // uses that word to say there is none, and a check that fires on
            // the denial is a check nobody can keep passing honestly.
            "analytics.track",
            "trackevent(",
            "captureevent(",
            "reportevent(",
        ];

        fn walk(dir: &std::path::Path, found: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                    continue;
                }
                let is_source = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| matches!(e, "rs" | "ts" | "tsx" | "swift" | "json"));
                if !is_source {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let lowered = text.to_lowercase();
                for needle in FORBIDDEN {
                    // This file names them all, by necessity.
                    if path.file_name().is_some_and(|n| n == "lib.rs") {
                        continue;
                    }
                    if lowered.contains(needle) {
                        found.push(format!("{}: {needle}", path.display()));
                    }
                }
            }
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");

        let mut found = Vec::new();
        for dir in ["src-tauri/src", "src", "sidecar/Sources"] {
            walk(&root.join(dir), &mut found);
        }

        assert!(
            found.is_empty(),
            "something that looks like telemetry appeared:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn no_telemetry_endpoint_is_reachable_from_the_privacy_snapshot() {
        // The privacy panel says "no telemetry". This asserts the claim rather
        // than trusting the sentence: if analytics were ever added, the field
        // would have to be flipped here, and flipping it fails the test.
        let snapshot = PrivacySnapshot {
            retention: retention::Retention::default(),
            audio_files: 0,
            audio_bytes: 0,
            generations: Vec::new(),
            telemetry: false,
        };
        assert!(!snapshot.telemetry, "the app started reporting telemetry");
    }

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
