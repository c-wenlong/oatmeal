//! Deciding which calendar entries look like meetings.
//!
//! A calendar is mostly not meetings. "Gym", "Focus time", "Lunch", "Dentist",
//! a week-long holiday — offering to record those trains the user to dismiss
//! the popup, which costs far more than missing one call. So this is a filter
//! with a bias toward silence.
//!
//! The rule from the roadmap: a meeting has a conferencing URL, **or** at least
//! two attendees, **or** an explicit location. Each of those says "other people
//! are involved", which is the actual signal.

use serde::{Deserialize, Serialize};

/// A calendar entry as the sidecar reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub title: Option<String>,
    pub starts_at: i64,
    pub ends_at: Option<i64>,
    pub location: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub attendee_count: i64,
}

/// Hosts whose links mean "this is a call".
///
/// Matched against the URL *and* the notes, because most calendar integrations
/// paste the join link into the body rather than the URL field.
const CONFERENCING_HOSTS: &[&str] = &[
    "zoom.us",
    "meet.google.com",
    "teams.microsoft.com",
    "teams.live.com",
    "webex.com",
    "whereby.com",
    "gotomeeting.com",
    "bluejeans.com",
    "chime.aws",
    "around.co",
    "meet.jit.si",
    "discord.gg",
    "slack.com/huddle",
];

/// Finds a conferencing link in the URL or the notes.
pub fn conferencing_url(event: &CalendarEvent) -> Option<String> {
    let haystacks = [event.url.as_deref(), event.notes.as_deref()];
    for text in haystacks.into_iter().flatten() {
        let lowered = text.to_lowercase();
        if let Some(host) = CONFERENCING_HOSTS.iter().find(|h| lowered.contains(**h)) {
            // Return the specific link when the field is one, otherwise just
            // report which service it is — the popup only needs to say "Zoom".
            if text.starts_with("http") && text.split_whitespace().count() == 1 {
                return Some(text.to_string());
            }
            return extract_url(text, host).or_else(|| Some((*host).to_string()));
        }
    }
    None
}

/// Pulls the first URL containing `host` out of a block of text.
fn extract_url(text: &str, host: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '"')
        .find(|token| token.to_lowercase().contains(host) && token.starts_with("http"))
        .map(|token| token.trim_end_matches(['.', ',', ')', ']']).to_string())
}

/// Whether this entry is worth offering to record.
///
/// Deliberately conservative. A solo block with no link, no location and no
/// other attendees is someone's focus time, and interrupting that is exactly
/// the behaviour that gets detection switched off.
pub fn is_meeting_shaped(event: &CalendarEvent) -> bool {
    if conferencing_url(event).is_some() {
        return true;
    }
    // Two, not one: the organiser counts as an attendee, so a solo hold often
    // arrives with exactly one.
    if event.attendee_count >= 2 {
        return true;
    }
    event
        .location
        .as_deref()
        .map(|l| !l.trim().is_empty())
        .unwrap_or(false)
}

/// When to raise a candidate for an event, given the configured lead time.
pub fn offer_at(event: &CalendarEvent, lead_ms: i64) -> i64 {
    event.starts_at - lead_ms
}

/// Events that should be offered now.
///
/// `already_offered` keeps a poll every five minutes from raising the same
/// meeting repeatedly — the queue would merge them, but re-offering something
/// the user has already dismissed is worse than merging.
pub fn due<'a>(
    events: &'a [CalendarEvent],
    now_ms: i64,
    lead_ms: i64,
    already_offered: &[String],
) -> Vec<&'a CalendarEvent> {
    events
        .iter()
        .filter(|event| is_meeting_shaped(event))
        .filter(|event| !already_offered.iter().any(|id| id == &event.id))
        // Within the lead window, and not already long finished. An event whose
        // end time has passed is history, however recently it appeared.
        .filter(|event| now_ms >= offer_at(event, lead_ms))
        .filter(|event| event.ends_at.map(|end| now_ms < end).unwrap_or(true))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str) -> CalendarEvent {
        CalendarEvent {
            id: id.into(),
            title: Some("Something".into()),
            starts_at: 1_000_000,
            ends_at: Some(2_000_000),
            location: None,
            url: None,
            notes: None,
            attendee_count: 0,
        }
    }

    #[test]
    fn a_zoom_link_makes_it_a_meeting() {
        let mut e = event("a");
        e.url = Some("https://acme.zoom.us/j/123456".into());
        assert!(is_meeting_shaped(&e));
        assert_eq!(
            conferencing_url(&e).as_deref(),
            Some("https://acme.zoom.us/j/123456")
        );
    }

    #[test]
    fn a_link_buried_in_the_notes_still_counts() {
        // Most calendar integrations paste the join link into the body, not the
        // URL field, so only checking `url` would miss the common case.
        let mut e = event("a");
        e.notes = Some("Join here: https://meet.google.com/abc-defg-hij\nDial-in: +1 555".into());
        assert!(is_meeting_shaped(&e));
        assert_eq!(
            conferencing_url(&e).as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn a_trailing_full_stop_is_not_part_of_the_link() {
        let mut e = event("a");
        e.notes = Some("Call at https://acme.zoom.us/j/999.".into());
        assert_eq!(
            conferencing_url(&e).as_deref(),
            Some("https://acme.zoom.us/j/999")
        );
    }

    #[test]
    fn two_attendees_make_it_a_meeting() {
        let mut e = event("a");
        e.attendee_count = 2;
        assert!(is_meeting_shaped(&e));
    }

    #[test]
    fn a_solo_hold_is_not_a_meeting() {
        // The most important negative. "Focus time" arrives with one attendee —
        // the organiser — and interrupting it is what makes people turn
        // detection off.
        let mut e = event("a");
        e.title = Some("Focus time".into());
        e.attendee_count = 1;
        assert!(!is_meeting_shaped(&e));
    }

    #[test]
    fn an_empty_calendar_block_is_not_a_meeting() {
        assert!(!is_meeting_shaped(&event("a")));
    }

    #[test]
    fn a_location_makes_it_a_meeting() {
        let mut e = event("a");
        e.location = Some("Room 4".into());
        assert!(is_meeting_shaped(&e));
    }

    #[test]
    fn a_blank_location_does_not_count() {
        // Some providers set the field to an empty string rather than omitting.
        let mut e = event("a");
        e.location = Some("   ".into());
        assert!(!is_meeting_shaped(&e));
    }

    #[test]
    fn an_event_is_offered_once_the_lead_time_is_reached() {
        let e = event("a");
        let lead = 90_000;
        assert!(due(std::slice::from_ref(&e), e.starts_at - lead - 1, lead, &[]).is_empty());
        assert_eq!(
            due(&[meeting()], meeting().starts_at - lead, lead, &[]).len(),
            1
        );
    }

    fn meeting() -> CalendarEvent {
        let mut e = event("m");
        e.attendee_count = 3;
        e
    }

    #[test]
    fn an_event_already_offered_is_not_offered_again() {
        // The calendar is polled every five minutes; without this the same
        // meeting would be raised repeatedly after being dismissed.
        let e = meeting();
        let offered = vec!["m".to_string()];
        assert!(due(std::slice::from_ref(&e), e.starts_at, 90_000, &offered).is_empty());
    }

    #[test]
    fn a_meeting_that_has_already_ended_is_not_offered() {
        let e = meeting();
        assert!(due(
            std::slice::from_ref(&e),
            e.ends_at.unwrap() + 1,
            90_000,
            &[]
        )
        .is_empty());
    }

    #[test]
    fn a_meeting_still_running_can_still_be_offered() {
        // Joining twenty minutes late is normal.
        let e = meeting();
        assert_eq!(
            due(std::slice::from_ref(&e), e.starts_at + 1_000, 90_000, &[]).len(),
            1
        );
    }

    #[test]
    fn an_event_with_no_end_time_is_still_offerable() {
        let mut e = meeting();
        e.ends_at = None;
        assert_eq!(
            due(std::slice::from_ref(&e), e.starts_at, 90_000, &[]).len(),
            1
        );
    }

    #[test]
    fn non_meetings_are_filtered_out_of_the_due_list() {
        let mut lunch = event("lunch");
        lunch.title = Some("Lunch".into());
        let events = vec![lunch, meeting()];
        let due_now = due(&events, meeting().starts_at, 90_000, &[]);
        assert_eq!(due_now.len(), 1);
        assert_eq!(due_now[0].id, "m");
    }

    #[test]
    fn a_case_mismatched_host_is_still_recognised() {
        let mut e = event("a");
        e.url = Some("https://ACME.ZOOM.US/j/1".into());
        assert!(is_meeting_shaped(&e));
    }
}
