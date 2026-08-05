//! Reading events, and turning them into the shape detection already speaks.
//!
//! The whole point of mapping onto `detect::CalendarEvent` is that nothing
//! downstream learns there are two calendar sources. The meeting-shaped
//! heuristic, the lead time, the dedup — all of it was written for EventKit and
//! keeps working unchanged.

use serde::Deserialize;

use super::token::TokenError;
use crate::detect::CalendarEvent;

const EVENTS_ENDPOINT: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";

#[derive(Debug, Deserialize)]
struct EventsResponse {
    #[serde(default)]
    items: Vec<GoogleEvent>,
}

#[derive(Debug, Deserialize)]
struct GoogleEvent {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    #[serde(rename = "hangoutLink")]
    hangout_link: Option<String>,
    #[serde(default)]
    start: Option<EventTime>,
    #[serde(default)]
    end: Option<EventTime>,
    #[serde(default)]
    attendees: Vec<Attendee>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventTime {
    /// RFC 3339, present for timed events.
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    /// Present instead for all-day events.
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Attendee {
    #[serde(default)]
    #[serde(rename = "responseStatus")]
    response_status: Option<String>,
}

/// Parses an RFC 3339 timestamp to epoch milliseconds.
///
/// Hand-rolled rather than adding a date crate: Google emits exactly one shape
/// here — `2026-08-03T14:30:00+01:00` or the same with `Z` — and the offset is
/// the only part that needs care.
pub fn parse_rfc3339(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() < 19 || bytes[10] != b'T' {
        return None;
    }
    let year: i64 = value.get(0..4)?.parse().ok()?;
    let month: i64 = value.get(5..7)?.parse().ok()?;
    let day: i64 = value.get(8..10)?.parse().ok()?;
    let hour: i64 = value.get(11..13)?.parse().ok()?;
    let minute: i64 = value.get(14..16)?.parse().ok()?;
    let second: i64 = value.get(17..19)?.parse().ok()?;

    let days = days_from_civil(year, month, day);
    let mut epoch = days * 86_400 + hour * 3_600 + minute * 60 + second;

    // The offset, if any. `Z` and a missing offset both mean UTC; anything else
    // is ±HH:MM and has to be subtracted to get back to UTC.
    let tail = &value[19..];
    if let Some(sign_at) = tail.find(['+', '-']) {
        let sign = if tail.as_bytes()[sign_at] == b'-' {
            -1
        } else {
            1
        };
        let offset = &tail[sign_at + 1..];
        let offset_hours: i64 = offset.get(0..2)?.parse().ok()?;
        let offset_minutes: i64 = offset.get(3..5).unwrap_or("00").parse().unwrap_or(0);
        epoch -= sign * (offset_hours * 3_600 + offset_minutes * 60);
    }
    Some(epoch * 1000)
}

/// Howard Hinnant's days-from-civil.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Maps a Google event onto the shape detection already understands.
///
/// Returns `None` for anything that cannot be a meeting: all-day entries,
/// cancelled events, and anything without a start time.
fn to_calendar_event(event: GoogleEvent) -> Option<CalendarEvent> {
    if event.status.as_deref() == Some("cancelled") {
        return None;
    }
    let start = event.start.as_ref()?;
    // All-day entries carry `date` and no `dateTime`. They are holidays,
    // birthdays and out-of-office blocks — never something to record. Checked
    // explicitly rather than inferred from a missing `dateTime`, so a future
    // shape that has neither is not silently treated as all-day.
    if start.date.is_some() {
        return None;
    }
    let starts_at = parse_rfc3339(start.date_time.as_deref()?)?;

    Some(CalendarEvent {
        id: event.id?,
        title: event.summary,
        starts_at,
        ends_at: event
            .end
            .as_ref()
            .and_then(|e| e.date_time.as_deref())
            .and_then(parse_rfc3339),
        location: event.location,
        // Meet links arrive in `hangoutLink`; everything else pastes a join URL
        // into the description. Both are handed to the existing extractor.
        url: event.hangout_link,
        notes: event.description,
        attendee_count: event
            .attendees
            .iter()
            // Declined attendees are not in the meeting, and counting them can
            // turn a one-person hold into a "meeting".
            .filter(|a| a.response_status.as_deref() != Some("declined"))
            .count() as i64,
    })
}

/// Upcoming events from the primary calendar.
pub async fn upcoming(
    http: &reqwest::Client,
    access_token: &str,
    now_ms: i64,
    horizon_ms: i64,
) -> Result<Vec<CalendarEvent>, TokenError> {
    let response = http
        .get(EVENTS_ENDPOINT)
        .bearer_auth(access_token)
        .query(&[
            ("timeMin", iso8601(now_ms)),
            ("timeMax", iso8601(now_ms + horizon_ms)),
            // Recurring events are expanded into their instances; without this
            // a weekly standup arrives as one master event with a recurrence
            // rule this code would have to interpret.
            ("singleEvents", "true".to_string()),
            ("orderBy", "startTime".to_string()),
            ("maxResults", "50".to_string()),
        ])
        .send()
        .await
        .map_err(|e| TokenError::Unreachable(e.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| TokenError::Unreachable(e.to_string()))?;

    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(TokenError::NeedsReauth);
    }
    if !status.is_success() {
        return Err(TokenError::Rejected(body.chars().take(200).collect()));
    }

    let parsed: EventsResponse =
        serde_json::from_str(&body).map_err(|e| TokenError::Malformed(e.to_string()))?;
    Ok(parsed
        .items
        .into_iter()
        .filter_map(to_calendar_event)
        .collect())
}

/// Epoch milliseconds as the RFC 3339 UTC string Google's query wants.
fn iso8601(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: serde_json::Value) -> Option<CalendarEvent> {
        to_calendar_event(serde_json::from_value(json).unwrap())
    }

    #[test]
    fn a_utc_timestamp_parses() {
        // 2023-11-14T22:13:20Z
        assert_eq!(
            parse_rfc3339("2023-11-14T22:13:20Z"),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn an_offset_is_applied() {
        // The same instant, expressed in +01:00, must produce the same epoch.
        assert_eq!(
            parse_rfc3339("2023-11-14T23:13:20+01:00"),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_rfc3339("2023-11-14T21:13:20-01:00"),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn a_missing_offset_is_treated_as_utc() {
        assert_eq!(
            parse_rfc3339("2023-11-14T22:13:20"),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn junk_does_not_panic() {
        assert_eq!(parse_rfc3339(""), None);
        assert_eq!(parse_rfc3339("2023-11-14"), None);
        assert_eq!(parse_rfc3339("not a date at all!!"), None);
    }

    #[test]
    fn a_timed_event_maps_across() {
        let mapped = event(serde_json::json!({
            "id": "evt-1",
            "summary": "Standup",
            "start": { "dateTime": "2023-11-14T22:13:20Z" },
            "end": { "dateTime": "2023-11-14T22:43:20Z" },
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "attendees": [{ "responseStatus": "accepted" }, { "responseStatus": "needsAction" }]
        }))
        .unwrap();

        assert_eq!(mapped.id, "evt-1");
        assert_eq!(mapped.title.as_deref(), Some("Standup"));
        assert_eq!(mapped.starts_at, 1_700_000_000_000);
        assert_eq!(mapped.attendee_count, 2);
        assert_eq!(
            mapped.url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn an_event_with_neither_shape_is_dropped_rather_than_guessed() {
        assert!(event(serde_json::json!({
            "id": "evt-1",
            "start": {}
        }))
        .is_none());
    }

    #[test]
    fn an_all_day_event_is_dropped() {
        // Holidays, birthdays, out-of-office. Never something to record.
        assert!(event(serde_json::json!({
            "id": "evt-1",
            "summary": "Public holiday",
            "start": { "date": "2023-11-14" },
            "end": { "date": "2023-11-15" }
        }))
        .is_none());
    }

    #[test]
    fn a_cancelled_event_is_dropped() {
        assert!(event(serde_json::json!({
            "id": "evt-1",
            "status": "cancelled",
            "start": { "dateTime": "2023-11-14T22:13:20Z" }
        }))
        .is_none());
    }

    #[test]
    fn a_declined_attendee_is_not_counted() {
        // Counting them turns a hold nobody accepted into a "meeting" and
        // produces a popup for something that is not happening.
        let mapped = event(serde_json::json!({
            "id": "evt-1",
            "start": { "dateTime": "2023-11-14T22:13:20Z" },
            "attendees": [
                { "responseStatus": "accepted" },
                { "responseStatus": "declined" }
            ]
        }))
        .unwrap();
        assert_eq!(mapped.attendee_count, 1);
    }

    #[test]
    fn the_description_is_carried_so_a_join_link_can_be_found() {
        // Zoom and Teams paste the link into the body, not `hangoutLink`.
        let mapped = event(serde_json::json!({
            "id": "evt-1",
            "start": { "dateTime": "2023-11-14T22:13:20Z" },
            "description": "Join: https://acme.zoom.us/j/123"
        }))
        .unwrap();
        assert!(mapped.notes.unwrap().contains("zoom.us"));
    }

    #[test]
    fn a_mapped_event_is_judged_by_the_existing_heuristic() {
        // The point of mapping onto the shared type: nothing downstream learns
        // there are two calendar sources.
        let mapped = event(serde_json::json!({
            "id": "evt-1",
            "summary": "Focus time",
            "start": { "dateTime": "2023-11-14T22:13:20Z" },
            "attendees": [{ "responseStatus": "accepted" }]
        }))
        .unwrap();
        assert!(!crate::detect::calendar::is_meeting_shaped(&mapped));

        let meeting = event(serde_json::json!({
            "id": "evt-2",
            "summary": "Standup",
            "start": { "dateTime": "2023-11-14T22:13:20Z" },
            "hangoutLink": "https://meet.google.com/abc"
        }))
        .unwrap();
        assert!(crate::detect::calendar::is_meeting_shaped(&meeting));
    }

    #[test]
    fn the_query_window_is_formatted_the_way_google_wants() {
        assert_eq!(iso8601(1_700_000_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn the_timestamp_helpers_round_trip() {
        for ms in [0_i64, 1_700_000_000_000, 1_785_000_000_000] {
            assert_eq!(parse_rfc3339(&iso8601(ms)), Some(ms));
        }
    }
}
