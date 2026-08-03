//! Deleting audio the user no longer needs.
//!
//! The promise this keeps is in SPEC §11: audio is kept *temporarily*. Without
//! something that actually deletes it, "auto-expire" is a column name, and a
//! year of meetings quietly becomes tens of gigabytes of recorded conversation
//! sitting on disk — the exact thing a local-first app is supposed to avoid.
//!
//! Two rules shape everything here:
//!
//! 1. **Only audio is ever deleted.** Transcripts, notes, panels and links are
//!    the durable record. Losing an hour of someone's meeting because a
//!    retention window ticked over would be indefensible, and the audio is only
//!    ever a re-listening aid.
//! 2. **A missing file is success, not failure.** The user may have deleted it
//!    themselves, or a previous sweep may have been interrupted after unlinking
//!    but before the database write. Treating that as an error would leave rows
//!    pointing at nothing forever.

pub mod sweep;

use serde::{Deserialize, Serialize};

pub use sweep::{purge_all, sweep};

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// How long audio is kept.
///
/// `Forever` is representable on purpose: someone recording interviews or
/// lectures has a legitimate reason to keep the source, and a settings screen
/// that silently caps them at 90 days would be lying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "days")]
pub enum Retention {
    Days(i64),
    Forever,
}

impl Default for Retention {
    fn default() -> Self {
        // SPEC §11's default.
        Retention::Days(7)
    }
}

impl Retention {
    /// When audio recorded now should expire. `None` means never.
    pub fn expires_at(self, now_ms: i64) -> Option<i64> {
        match self {
            Retention::Forever => None,
            // A zero or negative window would mean "delete audio the instant a
            // meeting ends", which is indistinguishable from a bug. One day is
            // the shortest setting that still lets someone re-listen.
            Retention::Days(days) => Some(now_ms + days.max(1) * DAY_MS),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "forever" => Some(Retention::Forever),
            other => other.parse().ok().map(Retention::Days),
        }
    }

    pub fn as_str(self) -> String {
        match self {
            Retention::Forever => "forever".into(),
            Retention::Days(days) => days.to_string(),
        }
    }
}

/// What a sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepReport {
    /// Files removed from disk.
    pub deleted: usize,
    /// Rows whose file was already gone. Not an error — see the module note.
    pub already_missing: usize,
    /// Bytes reclaimed.
    pub freed_bytes: u64,
}

impl SweepReport {
    pub fn touched(&self) -> usize {
        self.deleted + self.already_missing
    }
}

/// Whether a meeting's audio is due for deletion.
///
/// Split out because it is the whole policy, and a policy buried inside a
/// function that also touches the filesystem cannot be tested without one.
pub fn is_expired(expires_at: Option<i64>, now_ms: i64) -> bool {
    match expires_at {
        // No expiry recorded means "keep" — either the user chose forever, or
        // the row predates retention. Deleting on a null would turn a missing
        // value into data loss.
        None => false,
        Some(at) => now_ms >= at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_a_week() {
        assert_eq!(Retention::default(), Retention::Days(7));
    }

    #[test]
    fn a_window_produces_an_expiry() {
        let at = Retention::Days(7).expires_at(1_000).unwrap();
        assert_eq!(at, 1_000 + 7 * DAY_MS);
    }

    #[test]
    fn forever_produces_no_expiry() {
        assert_eq!(Retention::Forever.expires_at(1_000), None);
    }

    #[test]
    fn a_zero_day_window_still_keeps_audio_for_a_day() {
        // "Delete the moment a meeting ends" is indistinguishable from a bug,
        // and would destroy the recording before anyone could listen back.
        let at = Retention::Days(0).expires_at(0).unwrap();
        assert_eq!(at, DAY_MS);
        assert_eq!(Retention::Days(-5).expires_at(0), Some(DAY_MS));
    }

    #[test]
    fn expiry_is_inclusive_of_the_moment_it_passes() {
        assert!(!is_expired(Some(100), 99));
        assert!(is_expired(Some(100), 100));
        assert!(is_expired(Some(100), 101));
    }

    #[test]
    fn a_row_with_no_expiry_is_never_swept() {
        // Either the user chose "forever", or the row predates retention.
        // Treating a null as "expired" turns a missing value into data loss.
        assert!(!is_expired(None, i64::MAX));
    }

    #[test]
    fn retention_round_trips_through_its_stored_form() {
        for setting in [Retention::Days(7), Retention::Days(90), Retention::Forever] {
            assert_eq!(Retention::parse(&setting.as_str()), Some(setting));
        }
    }

    #[test]
    fn an_unparseable_setting_is_rejected_rather_than_guessed() {
        // Falling back to a default silently would change the user's retention
        // without telling them.
        assert_eq!(Retention::parse("soon"), None);
        assert_eq!(Retention::parse(""), None);
    }

    #[test]
    fn a_sweep_report_counts_everything_it_touched() {
        let report = SweepReport {
            deleted: 3,
            already_missing: 2,
            freed_bytes: 100,
        };
        assert_eq!(report.touched(), 5);
    }
}
