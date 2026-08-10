//! Wire protocol shared with the Swift sidecar.
//!
//! Mirrors `sidecar/Sources/SidecarProtocol/Protocol.swift`. Newline-delimited
//! JSON, one object per line. Kept as a pure module — no process handling, no
//! Tauri — so it can be exhaustively tested without spawning anything.

use serde::{Deserialize, Serialize};

/// Bumped whenever the wire format changes incompatibly. The supervisor refuses
/// a sidecar announcing a different version, so a stale binary fails loudly at
/// startup instead of silently dropping fields.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource {
    /// The user. Everything captured from the microphone.
    Mic,
    /// Everyone else. Everything the machine is playing.
    System,
}

impl AudioSource {
    /// Column value for `utterances.source`.
    pub fn as_str(self) -> &'static str {
        match self {
            AudioSource::Mic => "mic",
            AudioSource::System => "system",
        }
    }
}

/// TCC authorisation state for one capability.
///
/// `Undetermined` is kept distinct because it is the only state where prompting
/// can still succeed; after a denial macOS never shows the prompt again and the
/// UI must send the user to System Settings instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    Undetermined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    Downloading,
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum SidecarCommand {
    Start {
        meeting_id: String,
        sources: Vec<AudioSource>,
    },
    Stop,
    Ping,
    /// Begin capturing into the pre-roll buffer without writing to disk.
    Arm,
    /// Tear capture down entirely.
    Disarm,
    Permissions {
        #[serde(default)]
        request: bool,
    },
    /// Start or stop watching which apps hold the microphone (G21).
    WatchMic {
        enabled: bool,
    },
    /// Start or stop reading the calendar (G20).
    WatchCalendar {
        enabled: bool,
        #[serde(default)]
        request: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum SidecarEvent {
    /// First line the sidecar writes. Gates a successful spawn.
    Ready {
        version: String,
        #[serde(rename = "protocol")]
        protocol_version: u32,
    },
    /// In-flight text for the live UI. Never persisted — a later `Final`
    /// supersedes it.
    Partial {
        source: AudioSource,
        text: String,
        t0: i64,
        t1: i64,
    },
    /// Settled text. This is what becomes a row in `utterances`.
    Final {
        source: AudioSource,
        text: String,
        t0: i64,
        t1: i64,
        #[serde(default)]
        conf: Option<f64>,
    },
    Level {
        mic: f64,
        system: f64,
    },
    Stopped {
        #[serde(default)]
        audio_path: Option<String>,
        duration_ms: i64,
    },
    Error {
        message: String,
        #[serde(default)]
        fatal: bool,
    },
    Pong,
    /// Current TCC state. `needs_relaunch` means Screen Recording reads as
    /// granted but this process still holds a stale denial — macOS only hands a
    /// new grant to a freshly launched process.
    Permissions {
        microphone: PermissionState,
        screen_recording: PermissionState,
        #[serde(default)]
        needs_relaunch: bool,
    },
    /// ASR model lifecycle. `progress` is 0..=1 while downloading.
    Model {
        name: String,
        state: ModelState,
        #[serde(default)]
        progress: Option<f64>,
        #[serde(default)]
        message: Option<String>,
    },
    /// An app started or stopped using the microphone (G21). A report only —
    /// the policy about what may act on it lives in `detect::rules`.
    MicActivity {
        #[serde(default)]
        started: Vec<MicApp>,
        #[serde(default)]
        stopped: Vec<MicApp>,
    },
    /// The upcoming calendar window (G20), reported raw.
    CalendarEvents {
        #[serde(default)]
        events: Vec<crate::detect::CalendarEvent>,
        /// Every calendar the account holds, sent with the window rather than
        /// fetched separately — it changes about as often as the events do.
        #[serde(default)]
        calendars: Vec<crate::detect::calendar::CalendarSource>,
        #[serde(default)]
        authorized: bool,
    },
}

/// An app holding the microphone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicApp {
    pub pid: i64,
    /// Absent for processes with no bundle — daemons and scripts. Nothing can
    /// be ruled about those, so they are never acted on.
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

impl SidecarEvent {
    /// True when capture cannot start yet. Drives the "blocking pre-flight"
    /// requirement in G5 — the app must refuse to record rather than produce a
    /// silent, empty transcript.
    pub fn blocks_capture(&self) -> bool {
        match self {
            SidecarEvent::Permissions {
                microphone,
                screen_recording,
                needs_relaunch,
            } => {
                *needs_relaunch
                    || *microphone != PermissionState::Granted
                    || *screen_recording != PermissionState::Granted
            }
            _ => false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("malformed sidecar line: {source} (line: {line})")]
    Malformed {
        line: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("sidecar speaks protocol {found}, expected {expected}")]
    VersionMismatch { found: u32, expected: u32 },
}

/// Parses one line of sidecar stdout.
///
/// Blank lines yield `Ok(None)` rather than an error — the sidecar writes
/// diagnostics to stderr, but a stray newline shouldn't be treated as corruption.
pub fn parse_event(line: &str) -> Result<Option<SidecarEvent>, ProtocolError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|source| ProtocolError::Malformed {
            line: trimmed.to_string(),
            source,
        })
}

/// Serialises a command, newline included, ready to write to sidecar stdin.
pub fn encode_command(command: &SidecarCommand) -> String {
    let mut line = serde_json::to_string(command).expect("commands always serialize");
    line.push('\n');
    line
}

/// Verifies a `Ready` before the supervisor treats the sidecar as usable.
pub fn check_ready(event: &SidecarEvent) -> Result<(), ProtocolError> {
    match event {
        SidecarEvent::Ready {
            protocol_version, ..
        } if *protocol_version != PROTOCOL_VERSION => Err(ProtocolError::VersionMismatch {
            found: *protocol_version,
            expected: PROTOCOL_VERSION,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured verbatim from a real run of the Swift binary. If Swift's
    /// encoding drifts, these stop parsing — which is the whole point. Note the
    /// key order varies between runs (Swift's JSONEncoder is unordered), so
    /// nothing here may depend on ordering.
    const REAL_SIDECAR_OUTPUT: &[&str] = &[
        r#"{"protocol":1,"ev":"ready","version":"0.1.0"}"#,
        r#"{"ev":"level","mic":0.02,"system":0.31}"#,
        r#"{"t1":1500,"ev":"partial","source":"system","text":"so the deadline","t0":400}"#,
        r#"{"t1":3200,"conf":0.93,"ev":"final","source":"system","text":"So the deadline for the migration is the fourteenth.","t0":400}"#,
        r#"{"t1":5600,"conf":0.88,"ev":"final","source":"mic","text":"Got it, I'll own the rollback plan.","t0":3400}"#,
        r#"{"ev":"stopped","duration_ms":7900}"#,
    ];

    #[test]
    fn parses_real_sidecar_output() {
        let parsed: Vec<SidecarEvent> = REAL_SIDECAR_OUTPUT
            .iter()
            .map(|line| parse_event(line).expect("parse").expect("not blank"))
            .collect();

        assert!(matches!(
            parsed[0],
            SidecarEvent::Ready {
                protocol_version: 1,
                ..
            }
        ));
        assert!(matches!(parsed[1], SidecarEvent::Level { .. }));
        assert!(matches!(
            parsed[2],
            SidecarEvent::Partial {
                source: AudioSource::System,
                ..
            }
        ));

        match &parsed[3] {
            SidecarEvent::Final {
                source,
                text,
                t0,
                t1,
                conf,
            } => {
                assert_eq!(*source, AudioSource::System);
                assert!(text.contains("fourteenth"));
                assert_eq!((*t0, *t1), (400, 3200));
                assert_eq!(*conf, Some(0.93));
            }
            other => panic!("expected Final, got {other:?}"),
        }

        assert!(matches!(
            parsed[4],
            SidecarEvent::Final {
                source: AudioSource::Mic,
                ..
            }
        ));
        assert!(matches!(
            parsed[5],
            SidecarEvent::Stopped {
                audio_path: None,
                duration_ms: 7900
            }
        ));
    }

    /// Captured verbatim from the Swift binary answering `{"cmd":"permissions"}`.
    const REAL_PERMISSIONS_LINE: &str = r#"{"screen_recording":"granted","ev":"permissions","needs_relaunch":false,"microphone":"granted"}"#;

    #[test]
    fn parses_the_real_permissions_event() {
        let event = parse_event(REAL_PERMISSIONS_LINE).unwrap().unwrap();
        assert_eq!(
            event,
            SidecarEvent::Permissions {
                microphone: PermissionState::Granted,
                screen_recording: PermissionState::Granted,
                needs_relaunch: false,
            }
        );
        assert!(!event.blocks_capture());
    }

    #[test]
    fn any_missing_permission_blocks_capture() {
        // Recording with either capability missing yields a silent or half-empty
        // transcript, which is worse than refusing — so all three must block.
        for (mic, screen) in [
            (PermissionState::Denied, PermissionState::Granted),
            (PermissionState::Granted, PermissionState::Denied),
            (PermissionState::Undetermined, PermissionState::Granted),
        ] {
            let event = SidecarEvent::Permissions {
                microphone: mic,
                screen_recording: screen,
                needs_relaunch: false,
            };
            assert!(event.blocks_capture(), "{mic:?}/{screen:?} should block");
        }
    }

    #[test]
    fn a_stale_grant_blocks_capture_even_when_both_read_granted() {
        // The nastiest case: the checkbox is on, both report granted, and capture
        // still silently produces nothing until the app is relaunched.
        let event = SidecarEvent::Permissions {
            microphone: PermissionState::Granted,
            screen_recording: PermissionState::Granted,
            needs_relaunch: true,
        };
        assert!(event.blocks_capture());
    }

    #[test]
    fn permissions_command_defaults_to_query_not_prompt() {
        // A bare `{"cmd":"permissions"}` must not pop system dialogs at users.
        let cmd: SidecarCommand = serde_json::from_str(r#"{"cmd":"permissions"}"#).unwrap();
        assert_eq!(cmd, SidecarCommand::Permissions { request: false });
    }

    #[test]
    fn model_events_carry_progress_while_downloading() {
        let event = parse_event(
            r#"{"ev":"model","name":"small.en","state":"downloading","progress":0.42}"#,
        )
        .unwrap()
        .unwrap();
        match event {
            SidecarEvent::Model {
                name,
                state,
                progress,
                ..
            } => {
                assert_eq!(name, "small.en");
                assert_eq!(state, ModelState::Downloading);
                assert_eq!(progress, Some(0.42));
            }
            other => panic!("expected Model, got {other:?}"),
        }
    }

    #[test]
    fn both_sources_are_distinguished() {
        // If these ever collapsed to one value we'd lose speaker attribution
        // entirely, which is the reason for two capture streams.
        assert_ne!(AudioSource::Mic, AudioSource::System);
        assert_eq!(AudioSource::Mic.as_str(), "mic");
        assert_eq!(AudioSource::System.as_str(), "system");
    }

    #[test]
    fn blank_lines_are_ignored_not_errors() {
        assert!(parse_event("").unwrap().is_none());
        assert!(parse_event("   \n").unwrap().is_none());
    }

    #[test]
    fn malformed_json_is_reported_with_the_offending_line() {
        let err = parse_event("{not json").unwrap_err();
        assert!(matches!(err, ProtocolError::Malformed { .. }));
        assert!(err.to_string().contains("{not json"));
    }

    #[test]
    fn unknown_event_kind_is_rejected() {
        assert!(parse_event(r#"{"ev":"teleport"}"#).is_err());
    }

    #[test]
    fn final_without_confidence_parses() {
        // WhisperKit doesn't always give a confidence; a missing field must not
        // drop the utterance.
        let ev = parse_event(r#"{"ev":"final","source":"mic","text":"hi","t0":0,"t1":1}"#)
            .unwrap()
            .unwrap();
        assert!(matches!(ev, SidecarEvent::Final { conf: None, .. }));
    }

    #[test]
    fn commands_encode_one_per_line() {
        let line = encode_command(&SidecarCommand::Start {
            meeting_id: "m1".into(),
            sources: vec![AudioSource::Mic, AudioSource::System],
        });
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);

        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["cmd"], "start");
        assert_eq!(value["meeting_id"], "m1");
        assert_eq!(value["sources"][0], "mic");
        assert_eq!(value["sources"][1], "system");
    }

    #[test]
    fn stop_and_ping_encode_without_extra_fields() {
        let stop: serde_json::Value =
            serde_json::from_str(encode_command(&SidecarCommand::Stop).trim()).unwrap();
        assert_eq!(stop, serde_json::json!({"cmd": "stop"}));

        let ping: serde_json::Value =
            serde_json::from_str(encode_command(&SidecarCommand::Ping).trim()).unwrap();
        assert_eq!(ping, serde_json::json!({"cmd": "ping"}));
    }

    #[test]
    fn matching_protocol_version_is_accepted() {
        let ready = SidecarEvent::Ready {
            version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION,
        };
        assert!(check_ready(&ready).is_ok());
    }

    #[test]
    fn mismatched_protocol_version_is_rejected() {
        let ready = SidecarEvent::Ready {
            version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION + 1,
        };
        assert!(matches!(
            check_ready(&ready),
            Err(ProtocolError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn every_event_round_trips() {
        let events = vec![
            SidecarEvent::Ready {
                version: "0.1.0".into(),
                protocol_version: PROTOCOL_VERSION,
            },
            SidecarEvent::Partial {
                source: AudioSource::System,
                text: "hello".into(),
                t0: 0,
                t1: 10,
            },
            SidecarEvent::Final {
                source: AudioSource::Mic,
                text: "hello".into(),
                t0: 0,
                t1: 10,
                conf: Some(0.5),
            },
            SidecarEvent::Level {
                mic: 0.1,
                system: 0.2,
            },
            SidecarEvent::Stopped {
                audio_path: Some("/tmp/a.m4a".into()),
                duration_ms: 5,
            },
            SidecarEvent::Error {
                message: "boom".into(),
                fatal: true,
            },
            SidecarEvent::Pong,
        ];

        for event in events {
            let line = serde_json::to_string(&event).unwrap();
            let back = parse_event(&line).unwrap().unwrap();
            assert_eq!(back, event, "round trip changed the event");
        }
    }
}
