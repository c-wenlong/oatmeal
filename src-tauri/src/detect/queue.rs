//! One queue of "a meeting might be starting", fed from three directions.
//!
//! The hard requirement is **deduplication**. A calendar event at 10:00 and Zoom
//! taking the microphone at 09:59 are one meeting, and showing two popups for it
//! makes the feature feel broken in a way that no amount of accuracy elsewhere
//! makes up for. So candidates from different sources that land close together
//! in time collapse into one.
//!
//! The second requirement is that **nothing here records anything**. This
//! produces offers. Consent is a separate, explicit step.

use serde::{Deserialize, Serialize};

/// Where a candidate came from.
///
/// Ordered by how much it is trusted to name the meeting: a calendar event
/// knows the title and the attendees, a microphone activation knows only which
/// app started. When two merge, the better-informed one wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Something started using the microphone.
    Mic,
    /// A calendar event is about to start.
    Calendar,
    /// The user asked, from the tray or the hotkey. Always wins.
    Manual,
}

/// A possible meeting, waiting to be offered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub source: Source,
    /// Best known title. Empty when only the microphone fired.
    pub title: Option<String>,
    /// The app that triggered it, when a microphone activation did.
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    /// Calendar event this belongs to, when known.
    pub calendar_event_id: Option<String>,
    /// When this candidate was raised.
    pub at_ms: i64,
}

/// How far apart two candidates can be and still be the same meeting.
///
/// Generous, and deliberately so. People join early, calendars are set to the
/// hour while calls start at five past, and the popup is offered ahead of the
/// event anyway. Merging two things that were not the same meeting costs one
/// combined popup; failing to merge costs two popups for one call, which is the
/// thing users actually complain about.
pub const MERGE_WINDOW_MS: i64 = 5 * 60 * 1000;

/// How long an unanswered offer stays up before it counts as ignored.
pub const AUTO_DISMISS_MS: i64 = 60_000;

/// The pending candidate queue.
#[derive(Debug, Default)]
pub struct Queue {
    pending: Vec<Candidate>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending(&self) -> &[Candidate] {
        &self.pending
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Adds a candidate, merging it into an existing one when they are plainly
    /// the same meeting.
    ///
    /// Returns the id of the candidate that should be shown — either the new
    /// one, or the existing one it merged into.
    pub fn offer(&mut self, candidate: Candidate) -> String {
        if let Some(existing) = self
            .pending
            .iter_mut()
            .find(|pending| same_meeting(pending, &candidate))
        {
            merge_into(existing, candidate);
            return existing.id.clone();
        }
        let id = candidate.id.clone();
        self.pending.push(candidate);
        id
    }

    /// Removes a candidate, whatever the outcome. Returns it if it was there.
    pub fn resolve(&mut self, id: &str) -> Option<Candidate> {
        let index = self.pending.iter().position(|c| c.id == id)?;
        Some(self.pending.remove(index))
    }

    /// Drops candidates nobody answered in time, returning them.
    pub fn expire(&mut self, now_ms: i64) -> Vec<Candidate> {
        let (expired, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|c| now_ms - c.at_ms >= AUTO_DISMISS_MS);
        self.pending = live;
        expired
    }

    /// Forgets everything — used when detection is switched off, so a queue
    /// built up under the old settings cannot surface afterwards.
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

/// Whether two candidates are the same meeting.
fn same_meeting(a: &Candidate, b: &Candidate) -> bool {
    // Two candidates naming the same calendar event are the same meeting no
    // matter how far apart they were raised — an event that started an hour ago
    // and a mic activation now is someone joining late, not a second meeting.
    if let (Some(left), Some(right)) = (&a.calendar_event_id, &b.calendar_event_id) {
        return left == right;
    }
    (a.at_ms - b.at_ms).abs() <= MERGE_WINDOW_MS
}

/// Folds `incoming` into `existing`, keeping the better-informed fields.
fn merge_into(existing: &mut Candidate, incoming: Candidate) {
    // The source is upgraded, never downgraded: once a meeting is known to be a
    // calendar event, a later mic activation must not turn it back into an
    // anonymous "something is using your microphone".
    if incoming.source > existing.source {
        existing.source = incoming.source;
    }
    // A real title beats no title, and a calendar title beats an app name.
    if existing.title.is_none() {
        existing.title = incoming.title;
    }
    if existing.calendar_event_id.is_none() {
        existing.calendar_event_id = incoming.calendar_event_id;
    }
    // Knowing which app is on the call is useful even when the calendar named
    // the meeting — it is what "ignore this app" acts on.
    if existing.bundle_id.is_none() {
        existing.bundle_id = incoming.bundle_id;
        existing.app_name = incoming.app_name;
    }
    // Keep the earliest time: the popup's countdown should run from when we
    // first suspected a meeting, not from the last corroborating signal.
    existing.at_ms = existing.at_ms.min(incoming.at_ms);
}

/// What the user chose about an offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Start,
    Ignore,
    /// Ignore, and never offer this app again.
    IgnoreApp,
    /// Nobody answered.
    Expired,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, source: Source, at_ms: i64) -> Candidate {
        Candidate {
            id: id.into(),
            source,
            title: None,
            bundle_id: None,
            app_name: None,
            calendar_event_id: None,
            at_ms,
        }
    }

    #[test]
    fn a_lone_candidate_is_offered_as_itself() {
        let mut queue = Queue::new();
        let id = queue.offer(candidate("a", Source::Mic, 1_000));
        assert_eq!(id, "a");
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn a_calendar_event_and_a_mic_activation_make_one_popup() {
        // The headline requirement. Zoom opening a minute before the 10:00
        // event is one meeting, and two popups for it feels broken.
        let mut queue = Queue::new();
        queue.offer(Candidate {
            title: Some("Standup".into()),
            calendar_event_id: Some("evt-1".into()),
            ..candidate("cal", Source::Calendar, 600_000)
        });
        let id = queue.offer(Candidate {
            bundle_id: Some("us.zoom.xos".into()),
            app_name: Some("Zoom".into()),
            ..candidate("mic", Source::Mic, 660_000)
        });

        assert_eq!(queue.len(), 1, "should have merged into one candidate");
        assert_eq!(id, "cal", "the existing candidate is the one to show");

        let merged = &queue.pending()[0];
        assert_eq!(merged.title.as_deref(), Some("Standup"));
        assert_eq!(merged.bundle_id.as_deref(), Some("us.zoom.xos"));
    }

    #[test]
    fn merging_keeps_the_better_informed_source() {
        // A mic activation arriving after a calendar event must not turn a
        // named meeting back into "something is using your microphone".
        let mut queue = Queue::new();
        queue.offer(Candidate {
            title: Some("Standup".into()),
            calendar_event_id: Some("evt-1".into()),
            ..candidate("cal", Source::Calendar, 600_000)
        });
        queue.offer(candidate("mic", Source::Mic, 610_000));

        assert_eq!(queue.pending()[0].source, Source::Calendar);
    }

    #[test]
    fn a_mic_activation_first_is_upgraded_by_the_calendar() {
        let mut queue = Queue::new();
        queue.offer(Candidate {
            bundle_id: Some("us.zoom.xos".into()),
            ..candidate("mic", Source::Mic, 600_000)
        });
        queue.offer(Candidate {
            title: Some("Standup".into()),
            calendar_event_id: Some("evt-1".into()),
            ..candidate("cal", Source::Calendar, 620_000)
        });

        assert_eq!(queue.len(), 1);
        let merged = &queue.pending()[0];
        assert_eq!(merged.source, Source::Calendar);
        assert_eq!(merged.title.as_deref(), Some("Standup"));
        assert_eq!(merged.bundle_id.as_deref(), Some("us.zoom.xos"));
    }

    #[test]
    fn meetings_far_apart_in_time_stay_separate() {
        // Back-to-back calls are two meetings, and collapsing them would lose
        // the second one entirely.
        let mut queue = Queue::new();
        queue.offer(candidate("a", Source::Mic, 0));
        queue.offer(candidate("b", Source::Mic, MERGE_WINDOW_MS + 1));
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn the_same_calendar_event_merges_however_late_it_is() {
        // Joining an hour into a long meeting is still that meeting.
        let mut queue = Queue::new();
        queue.offer(Candidate {
            calendar_event_id: Some("evt-1".into()),
            ..candidate("cal", Source::Calendar, 0)
        });
        queue.offer(Candidate {
            calendar_event_id: Some("evt-1".into()),
            ..candidate("late", Source::Mic, 3_600_000)
        });
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn different_calendar_events_never_merge_even_when_adjacent() {
        // Two 30-minute meetings back to back share a boundary; they are not
        // one meeting, and the ids say so.
        let mut queue = Queue::new();
        queue.offer(Candidate {
            calendar_event_id: Some("evt-1".into()),
            ..candidate("a", Source::Calendar, 0)
        });
        queue.offer(Candidate {
            calendar_event_id: Some("evt-2".into()),
            ..candidate("b", Source::Calendar, 60_000)
        });
        assert_eq!(queue.len(), 2, "adjacent events were wrongly merged");
    }

    #[test]
    fn merging_keeps_the_earliest_time_so_the_countdown_is_honest() {
        let mut queue = Queue::new();
        queue.offer(candidate("a", Source::Mic, 600_000));
        queue.offer(candidate("b", Source::Calendar, 630_000));
        assert_eq!(queue.pending()[0].at_ms, 600_000);
    }

    #[test]
    fn resolving_removes_a_candidate() {
        let mut queue = Queue::new();
        queue.offer(candidate("a", Source::Mic, 0));
        assert!(queue.resolve("a").is_some());
        assert!(queue.is_empty());
        assert!(queue.resolve("a").is_none(), "resolved twice");
    }

    #[test]
    fn unanswered_offers_expire() {
        let mut queue = Queue::new();
        queue.offer(candidate("a", Source::Mic, 0));
        assert!(queue.expire(AUTO_DISMISS_MS - 1).is_empty());

        let expired = queue.expire(AUTO_DISMISS_MS);
        assert_eq!(expired.len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn expiry_leaves_younger_candidates_alone() {
        let mut queue = Queue::new();
        queue.offer(candidate("old", Source::Mic, 0));
        // Far enough apart not to merge.
        queue.offer(candidate("new", Source::Mic, MERGE_WINDOW_MS + 1));

        let expired = queue.expire(MERGE_WINDOW_MS + 2);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, "old");
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn a_manual_request_outranks_everything() {
        let mut queue = Queue::new();
        queue.offer(candidate("mic", Source::Mic, 0));
        queue.offer(candidate("manual", Source::Manual, 1_000));
        assert_eq!(queue.pending()[0].source, Source::Manual);
    }

    #[test]
    fn clearing_empties_the_queue() {
        // Turning detection off must not leave offers to surface later.
        let mut queue = Queue::new();
        queue.offer(candidate("a", Source::Mic, 0));
        queue.clear();
        assert!(queue.is_empty());
    }
}
