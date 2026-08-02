//! The meeting lifecycle, as an explicit state machine.
//!
//! Before this existed, "is a meeting recording?" was an `Option<String>` that
//! several places set and cleared independently. That made two things
//! impossible to reason about: what happens when `stop` is requested but the
//! sidecar never confirms, and what a second `start` should do while the first
//! is still finalising.
//!
//! The transition function is pure, so every one of those questions is answered
//! by a test rather than by reading call sites.

use serde::Serialize;

/// Where a meeting is in its life.
///
/// `Processing` is a real state, not a formality: `stop` only *asks* the sidecar
/// to finish. The audio file is not complete, and the last utterances have not
/// arrived, until it answers. Treating that gap as "complete" would report a
/// truncated recording as finished.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MeetingState {
    /// Nothing is being captured at all.
    #[default]
    Idle,
    /// Capturing into the pre-roll buffer; nothing on disk.
    Armed,
    Recording {
        meeting_id: String,
    },
    /// Stop requested; waiting for the sidecar to finalise the file.
    Processing {
        meeting_id: String,
    },
}

impl MeetingState {
    /// The meeting utterances should be attributed to, if any.
    ///
    /// Deliberately `Some` while `Processing`: audio settled before the stop is
    /// still arriving, and it belongs to that meeting rather than nowhere.
    pub fn active_meeting(&self) -> Option<&str> {
        match self {
            MeetingState::Recording { meeting_id } | MeetingState::Processing { meeting_id } => {
                Some(meeting_id)
            }
            _ => None,
        }
    }

    pub fn is_recording(&self) -> bool {
        matches!(self, MeetingState::Recording { .. })
    }

    /// Whether capture hardware is running (armed or beyond).
    pub fn is_capturing(&self) -> bool {
        !matches!(self, MeetingState::Idle)
    }
}

/// Things that move a meeting along.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingEvent {
    Armed,
    Started {
        meeting_id: String,
    },
    /// The user asked to stop.
    StopRequested,
    /// The sidecar confirmed the recording is finalised.
    SidecarStopped,
    Disarmed,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    #[error("a meeting is already recording")]
    AlreadyRecording,
    #[error("the previous meeting is still finalising")]
    StillProcessing,
    #[error("nothing is recording")]
    NotRecording,
}

/// The whole lifecycle, in one place.
///
/// Unexpected events are errors rather than silent no-ops — a `stop` with
/// nothing recording means a caller is confused, and swallowing it hides the
/// bug until a recording goes missing.
pub fn next(state: &MeetingState, event: MeetingEvent) -> Result<MeetingState, TransitionError> {
    use MeetingEvent as E;
    use MeetingState as S;

    match (state, event) {
        // Arming is idempotent: the UI arms on meeting detection and again on a
        // manual start, and the second one must not be an error.
        (S::Idle, E::Armed) => Ok(S::Armed),
        (S::Armed, E::Armed) => Ok(S::Armed),
        (S::Recording { .. } | S::Processing { .. }, E::Armed) => Ok(state.clone()),

        // Starting from idle implicitly arms — the capture engine does the same.
        (S::Idle | S::Armed, E::Started { meeting_id }) => Ok(S::Recording { meeting_id }),
        (S::Recording { .. }, E::Started { .. }) => Err(TransitionError::AlreadyRecording),
        (S::Processing { .. }, E::Started { .. }) => Err(TransitionError::StillProcessing),

        (S::Recording { meeting_id }, E::StopRequested) => Ok(S::Processing {
            meeting_id: meeting_id.clone(),
        }),
        // Asking twice is harmless; the first request is already in flight.
        (S::Processing { .. }, E::StopRequested) => Ok(state.clone()),
        (S::Idle | S::Armed, E::StopRequested) => Err(TransitionError::NotRecording),

        // The sidecar confirming a stop leaves capture armed, so a following
        // meeting still gets its pre-roll.
        (S::Processing { .. }, E::SidecarStopped) => Ok(S::Armed),
        // A stop we did not ask for means the sidecar restarted mid-recording.
        // The meeting is over either way; do not keep attributing audio to it.
        (S::Recording { .. }, E::SidecarStopped) => Ok(S::Armed),
        (S::Idle | S::Armed, E::SidecarStopped) => Ok(state.clone()),

        (S::Idle | S::Armed, E::Disarmed) => Ok(S::Idle),
        // Disarming mid-recording would drop audio silently.
        (S::Recording { .. }, E::Disarmed) => Err(TransitionError::AlreadyRecording),
        (S::Processing { .. }, E::Disarmed) => Err(TransitionError::StillProcessing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording(id: &str) -> MeetingState {
        MeetingState::Recording {
            meeting_id: id.into(),
        }
    }

    fn processing(id: &str) -> MeetingState {
        MeetingState::Processing {
            meeting_id: id.into(),
        }
    }

    #[test]
    fn a_meeting_runs_the_full_happy_path() {
        let state = MeetingState::default();
        assert_eq!(state, MeetingState::Idle);

        let state = next(&state, MeetingEvent::Armed).unwrap();
        assert_eq!(state, MeetingState::Armed);

        let state = next(
            &state,
            MeetingEvent::Started {
                meeting_id: "m1".into(),
            },
        )
        .unwrap();
        assert_eq!(state, recording("m1"));

        let state = next(&state, MeetingEvent::StopRequested).unwrap();
        assert_eq!(state, processing("m1"));

        let state = next(&state, MeetingEvent::SidecarStopped).unwrap();
        // Stays armed so the next meeting still gets a pre-roll.
        assert_eq!(state, MeetingState::Armed);
    }

    #[test]
    fn stopping_does_not_complete_until_the_sidecar_confirms() {
        // `stop` only asks. The file is not finalised and late utterances have
        // not arrived, so calling it complete here would report a truncated
        // recording as finished.
        let state = next(&recording("m1"), MeetingEvent::StopRequested).unwrap();
        assert_eq!(state, processing("m1"));
        assert_eq!(state.active_meeting(), Some("m1"));
        assert!(!state.is_recording());
    }

    #[test]
    fn utterances_still_belong_to_the_meeting_while_it_finalises() {
        // Audio settled just before the stop arrives during Processing; it
        // belongs to that meeting, not to nothing.
        assert_eq!(processing("m1").active_meeting(), Some("m1"));
    }

    #[test]
    fn a_second_start_is_refused_while_recording() {
        let err = next(
            &recording("m1"),
            MeetingEvent::Started {
                meeting_id: "m2".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err, TransitionError::AlreadyRecording);
    }

    #[test]
    fn a_start_is_refused_while_the_previous_meeting_finalises() {
        // Allowing it would attribute the tail of meeting one to meeting two.
        let err = next(
            &processing("m1"),
            MeetingEvent::Started {
                meeting_id: "m2".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err, TransitionError::StillProcessing);
    }

    #[test]
    fn a_new_meeting_can_start_once_the_previous_one_finished() {
        let state = next(&processing("m1"), MeetingEvent::SidecarStopped).unwrap();
        let state = next(
            &state,
            MeetingEvent::Started {
                meeting_id: "m2".into(),
            },
        )
        .unwrap();
        assert_eq!(state, recording("m2"));
    }

    #[test]
    fn arming_is_idempotent() {
        // The UI arms on detection and again on a manual start.
        let state = next(&MeetingState::Armed, MeetingEvent::Armed).unwrap();
        assert_eq!(state, MeetingState::Armed);
    }

    #[test]
    fn arming_never_disturbs_a_recording() {
        let state = next(&recording("m1"), MeetingEvent::Armed).unwrap();
        assert_eq!(
            state,
            recording("m1"),
            "arming interrupted a live recording"
        );
    }

    #[test]
    fn stopping_twice_is_harmless() {
        let state = next(&processing("m1"), MeetingEvent::StopRequested).unwrap();
        assert_eq!(state, processing("m1"));
    }

    #[test]
    fn stopping_with_nothing_recording_is_an_error_not_a_no_op() {
        // Swallowing it would hide a confused caller until a recording went
        // missing.
        assert_eq!(
            next(&MeetingState::Armed, MeetingEvent::StopRequested).unwrap_err(),
            TransitionError::NotRecording
        );
        assert_eq!(
            next(&MeetingState::Idle, MeetingEvent::StopRequested).unwrap_err(),
            TransitionError::NotRecording
        );
    }

    #[test]
    fn a_sidecar_restart_mid_recording_ends_the_meeting() {
        // The supervisor restarts a dead sidecar, which emits `stopped` we never
        // asked for. Continuing to attribute audio to the meeting would splice
        // two recordings together.
        let state = next(&recording("m1"), MeetingEvent::SidecarStopped).unwrap();
        assert_eq!(state, MeetingState::Armed);
        assert_eq!(state.active_meeting(), None);
    }

    #[test]
    fn an_unexpected_stop_while_idle_is_ignored() {
        // Harmless: the sidecar reports its state on restart.
        assert_eq!(
            next(&MeetingState::Idle, MeetingEvent::SidecarStopped).unwrap(),
            MeetingState::Idle
        );
    }

    #[test]
    fn disarming_mid_recording_is_refused() {
        // It would drop audio the user believes is being captured.
        assert_eq!(
            next(&recording("m1"), MeetingEvent::Disarmed).unwrap_err(),
            TransitionError::AlreadyRecording
        );
        assert_eq!(
            next(&processing("m1"), MeetingEvent::Disarmed).unwrap_err(),
            TransitionError::StillProcessing
        );
    }

    #[test]
    fn disarming_from_armed_or_idle_reaches_idle() {
        assert_eq!(
            next(&MeetingState::Armed, MeetingEvent::Disarmed).unwrap(),
            MeetingState::Idle
        );
        assert_eq!(
            next(&MeetingState::Idle, MeetingEvent::Disarmed).unwrap(),
            MeetingState::Idle
        );
    }

    #[test]
    fn only_recording_counts_as_recording() {
        assert!(recording("m1").is_recording());
        assert!(!processing("m1").is_recording());
        assert!(!MeetingState::Armed.is_recording());
    }

    #[test]
    fn capturing_covers_everything_except_idle() {
        assert!(!MeetingState::Idle.is_capturing());
        assert!(MeetingState::Armed.is_capturing());
        assert!(recording("m1").is_capturing());
        assert!(processing("m1").is_capturing());
    }

    #[test]
    fn no_event_can_panic_from_any_state() {
        // Exhaustiveness: the machine must have an answer for every pair, even
        // if that answer is an error.
        let states = [
            MeetingState::Idle,
            MeetingState::Armed,
            recording("m1"),
            processing("m1"),
        ];
        let events = [
            MeetingEvent::Armed,
            MeetingEvent::Started {
                meeting_id: "m2".into(),
            },
            MeetingEvent::StopRequested,
            MeetingEvent::SidecarStopped,
            MeetingEvent::Disarmed,
        ];

        for state in &states {
            for event in &events {
                let _ = next(state, event.clone());
            }
        }
    }

    #[test]
    fn the_state_serialises_for_the_frontend() {
        let json = serde_json::to_value(recording("m1")).unwrap();
        assert_eq!(json["state"], "recording");
        assert_eq!(json["meeting_id"], "m1");
        assert_eq!(
            serde_json::to_value(MeetingState::Idle).unwrap()["state"],
            "idle"
        );
    }
}
