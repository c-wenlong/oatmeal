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
    /// Which calendar it came from. Optional: an event with no calendar is not
    /// worth discarding, and the Google path has only one calendar anyway.
    #[serde(default)]
    pub calendar_id: Option<String>,
}

/// A calendar the account holds.
///
/// Carries its colour because the list is unreadable without it — two accounts
/// both with a "Work" calendar are told apart by the dot, exactly as they are
/// in Calendar.app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSource {
    pub id: String,
    pub title: String,
    /// `#rrggbb`, or absent when EventKit has no colour for it.
    #[serde(default)]
    pub color: Option<String>,
    /// Whether its events are considered. Decided here, not by the sidecar.
    #[serde(default = "yes")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}

/// Drops events from calendars the user has hidden.
///
/// Hiding is stored as the set of *hidden* ids rather than visible ones, so a
/// calendar added after the choice was made shows up by default. The other way
/// round, every new calendar would arrive silently switched off.
pub fn visible<'a>(
    events: &'a [CalendarEvent],
    hidden: &std::collections::HashSet<String>,
) -> Vec<&'a CalendarEvent> {
    events
        .iter()
        .filter(|event| match &event.calendar_id {
            Some(id) => !hidden.contains(id),
            // No calendar means nothing to hide it by. Dropping it would make
            // the Google path — which reports no calendar id — vanish entirely.
            None => true,
        })
        .collect()
}

/// The whole list a user sees: EventKit's calendars, plus the connected
/// account that is not one.
///
/// The Google source is not an EventKit calendar and never will be — its scope
/// cannot enumerate calendars — but leaving it out of a list headed "visible
/// calendars" makes the one source the user explicitly connected the one source
/// they cannot find. It is one row because there is nothing to expand it into.
///
/// Taking it as a parameter rather than reaching for app state keeps this
/// testable, which is the point: the first attempt at this lived inside the
/// Tauri command, was silently lost in an edit, and nothing failed.
pub fn sources_for_display(
    eventkit: &[CalendarSource],
    hidden: &std::collections::HashSet<String>,
    google: &[crate::gcal::calendars::GoogleCalendar],
) -> Vec<CalendarSource> {
    let mut list = with_visibility(eventkit, hidden);
    list.extend(google.iter().map(|calendar| {
        CalendarSource {
            id: crate::gcal::calendars::source_id(&calendar.id),
            title: calendar.title.clone(),
            // The account's own colour for it. Falls back to Google blue, so a
            // calendar with no colour still reads as a row rather than a gap.
            color: Some(
                calendar
                    .color
                    .clone()
                    .unwrap_or_else(|| "#4285f4".to_string()),
            ),
            visible: !hidden.contains(&crate::gcal::calendars::source_id(&calendar.id)),
        }
    }));
    list
}

/// The calendar list, with the user's choices applied.
pub fn with_visibility(
    sources: &[CalendarSource],
    hidden: &std::collections::HashSet<String>,
) -> Vec<CalendarSource> {
    sources
        .iter()
        .map(|source| CalendarSource {
            visible: !hidden.contains(&source.id),
            ..source.clone()
        })
        .collect()
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
/// `include_solo` relaxes the meeting-shape rule to let through an entry with
/// no link, no location and nobody else on it. Off by default, and it should
/// stay off for most people: those entries are focus time, and offering to
/// record them is what teaches a user to dismiss the popup on sight.
pub fn due<'a>(
    events: &'a [CalendarEvent],
    now_ms: i64,
    lead_ms: i64,
    already_offered: &[String],
    include_solo: bool,
) -> Vec<&'a CalendarEvent> {
    events
        .iter()
        .filter(|event| include_solo || is_meeting_shaped(event))
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
            calendar_id: None,
        }
    }

    fn source(id: &str) -> CalendarSource {
        CalendarSource {
            id: id.into(),
            title: id.into(),
            color: None,
            visible: true,
        }
    }

    fn hidden(ids: &[&str]) -> std::collections::HashSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    fn google(id: &str, title: &str) -> crate::gcal::calendars::GoogleCalendar {
        crate::gcal::calendars::GoogleCalendar {
            id: id.into(),
            title: title.into(),
            color: Some("#f83a22".into()),
            primary: false,
        }
    }

    #[test]
    fn every_calendar_in_the_account_gets_its_own_row() {
        // Not one row saying "Google". The account has several calendars and
        // the user wants to choose between them.
        let list = sources_for_display(
            &[],
            &hidden(&[]),
            &[
                google("me@example.com", "Personal"),
                google("w@g.com", "Work"),
            ],
        );
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "Personal");
        assert_eq!(list[1].title, "Work");
    }

    #[test]
    fn a_google_calendar_switched_off_stays_off() {
        // The same hidden set as the local calendars, so one rule covers both.
        let list = sources_for_display(
            &[],
            &hidden(&["google:w@g.com"]),
            &[
                google("me@example.com", "Personal"),
                google("w@g.com", "Work"),
            ],
        );
        assert!(list[0].visible);
        assert!(!list[1].visible, "the hidden Google calendar should be off");
    }

    #[test]
    fn google_sits_alongside_the_local_calendars() {
        let list = sources_for_display(
            &[source("work"), source("personal")],
            &hidden(&["personal"]),
            &[google("me@example.com", "Personal")],
        );
        assert_eq!(list.len(), 3);
        assert!(list[0].visible, "work was not hidden");
        assert!(!list[1].visible, "personal was hidden");
        assert!(list[2].visible, "google was not hidden");
    }

    #[test]
    fn no_account_means_no_google_rows() {
        assert!(sources_for_display(&[], &hidden(&[]), &[]).is_empty());
    }

    #[test]
    fn a_hidden_calendar_contributes_nothing() {
        let mut work = event("a");
        work.calendar_id = Some("work".into());
        let mut personal = event("b");
        personal.calendar_id = Some("personal".into());

        let events = [work, personal];
        let kept = visible(&events, &hidden(&["personal"]));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "a");
    }

    #[test]
    fn an_event_with_no_calendar_survives() {
        // The Google path reports no calendar id at all. Dropping those would
        // make that whole source vanish the moment anything was hidden.
        let orphan = [event("a")];
        assert_eq!(visible(&orphan, &hidden(&["work"])).len(), 1);
    }

    #[test]
    fn a_new_calendar_is_visible_without_being_chosen() {
        // Hidden ids are stored rather than visible ones precisely for this: a
        // calendar added after the choice would otherwise arrive switched off,
        // silently, and the user would never know it existed.
        let list = with_visibility(&[source("added-today")], &hidden(&["work"]));
        assert!(list[0].visible);
    }

    #[test]
    fn the_list_reports_what_the_user_switched_off() {
        let list = with_visibility(&[source("work"), source("personal")], &hidden(&["work"]));
        assert!(!list[0].visible);
        assert!(list[1].visible);
    }

    #[test]
    fn a_solo_hold_is_offered_only_when_asked_for() {
        // The default has to stay conservative: offering to record focus time
        // is what teaches someone to dismiss the popup without reading it.
        let e = event("a");
        assert!(!is_meeting_shaped(&e));
        let at = e.starts_at;
        assert!(due(std::slice::from_ref(&e), at, 90_000, &[], false).is_empty());
        assert_eq!(
            due(std::slice::from_ref(&e), at, 90_000, &[], true).len(),
            1
        );
    }

    #[test]
    fn including_solo_events_does_not_disable_the_other_filters() {
        // It relaxes meeting-shape and nothing else — an event already finished
        // is still history, and one already offered is still not re-offered.
        let e = event("a");
        assert!(due(
            std::slice::from_ref(&e),
            e.ends_at.unwrap() + 1,
            90_000,
            &[],
            true
        )
        .is_empty());
        assert!(due(
            std::slice::from_ref(&e),
            e.starts_at,
            90_000,
            &["a".to_string()],
            true
        )
        .is_empty());
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
        assert!(due(
            std::slice::from_ref(&e),
            e.starts_at - lead - 1,
            lead,
            &[],
            false
        )
        .is_empty());
        assert_eq!(
            due(&[meeting()], meeting().starts_at - lead, lead, &[], false).len(),
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
        assert!(due(
            std::slice::from_ref(&e),
            e.starts_at,
            90_000,
            &offered,
            false
        )
        .is_empty());
    }

    #[test]
    fn a_meeting_that_has_already_ended_is_not_offered() {
        let e = meeting();
        assert!(due(
            std::slice::from_ref(&e),
            e.ends_at.unwrap() + 1,
            90_000,
            &[],
            false
        )
        .is_empty());
    }

    #[test]
    fn a_meeting_still_running_can_still_be_offered() {
        // Joining twenty minutes late is normal.
        let e = meeting();
        assert_eq!(
            due(
                std::slice::from_ref(&e),
                e.starts_at + 1_000,
                90_000,
                &[],
                false
            )
            .len(),
            1
        );
    }

    #[test]
    fn an_event_with_no_end_time_is_still_offerable() {
        let mut e = meeting();
        e.ends_at = None;
        assert_eq!(
            due(std::slice::from_ref(&e), e.starts_at, 90_000, &[], false).len(),
            1
        );
    }

    #[test]
    fn non_meetings_are_filtered_out_of_the_due_list() {
        let mut lunch = event("lunch");
        lunch.title = Some("Lunch".into());
        let events = vec![lunch, meeting()];
        let due_now = due(&events, meeting().starts_at, 90_000, &[], false);
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
