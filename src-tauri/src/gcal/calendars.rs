//! The calendars in the connected account.
//!
//! `GET https://www.googleapis.com/calendar/v3/users/me/calendarList`
//! (<https://developers.google.com/workspace/calendar/api/v3/reference/calendarList/list>)
//!
//! This needs a scope `calendar.events.readonly` does not grant. The narrowest
//! that works is `calendar.calendarlist.readonly` — the names and colours of
//! the calendars, and nothing else. `calendar.readonly` would also work and
//! would additionally hand over every event body in the account, which is more
//! than a list of names is worth.

use serde::{Deserialize, Serialize};

pub const CALENDAR_LIST_ENDPOINT: &str =
    "https://www.googleapis.com/calendar/v3/users/me/calendarList";

/// One calendar, as the account describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCalendar {
    /// The calendar's own id. For the primary calendar this is the account's
    /// email address, which is how the UI can name the connected account
    /// without asking for a profile scope on top.
    pub id: String,
    pub title: String,
    pub color: Option<String>,
    pub primary: bool,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default)]
    items: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: String,
    #[serde(default)]
    summary: Option<String>,
    /// What the user renamed it to, which is what they will look for.
    #[serde(default)]
    summary_override: Option<String>,
    #[serde(default)]
    background_color: Option<String>,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    deleted: bool,
}

/// Parses the response into the calendars worth showing.
///
/// Deleted entries are dropped: they are still in the payload so a syncing
/// client can remove them, and showing one offers a switch over a calendar
/// that no longer exists.
pub fn parse(body: &str) -> Result<Vec<GoogleCalendar>, serde_json::Error> {
    let response: ListResponse = serde_json::from_str(body)?;
    Ok(response
        .items
        .into_iter()
        .filter(|entry| !entry.deleted)
        .map(|entry| GoogleCalendar {
            title: entry
                // The user's own name for it wins over Google's. Someone who
                // renamed "kai@example.com" to "Work" is looking for "Work".
                .summary_override
                .or(entry.summary)
                .unwrap_or_else(|| entry.id.clone()),
            color: entry.background_color,
            primary: entry.primary,
            id: entry.id,
        })
        .collect())
}

/// The account these calendars belong to.
///
/// The primary calendar's id is the account's email address. Read from what
/// the list already returned rather than asking for an identity scope on top
/// — a summary screen is not worth widening what the app may see.
///
/// `None` when nothing in the list is marked primary, which is possible for a
/// delegated or service account. Saying nothing beats guessing at an identity.
pub fn account_email(calendars: &[GoogleCalendar]) -> Option<String> {
    calendars
        .iter()
        .find(|calendar| calendar.primary)
        .map(|calendar| calendar.id.clone())
        .filter(|id| id.contains('@'))
}

/// The stable id a calendar is stored and toggled by.
///
/// Namespaced, because these share a list with EventKit's calendars and an
/// account whose calendar id collided with a local one would silently toggle
/// the wrong row.
pub fn source_id(calendar_id: &str) -> String {
    format!("google:{calendar_id}")
}

/// Whether a source id came from Google, and which calendar it names.
pub fn calendar_id_of(source_id: &str) -> Option<&str> {
    source_id.strip_prefix("google:")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
      "kind": "calendar#calendarList",
      "items": [
        {"id": "chenwenlongofficial@gmail.com", "summary": "chenwenlongofficial@gmail.com",
         "backgroundColor": "#9fe1e7", "primary": true, "accessRole": "owner"},
        {"id": "abc123@group.calendar.google.com", "summary": "Work",
         "backgroundColor": "#f83a22", "accessRole": "writer"},
        {"id": "renamed@group.calendar.google.com", "summary": "Original",
         "summaryOverride": "My name for it", "accessRole": "reader"},
        {"id": "gone@group.calendar.google.com", "summary": "Removed", "deleted": true}
      ]
    }"##;

    #[test]
    fn every_calendar_in_the_account_is_listed() {
        // The whole point: one row per calendar, not one row for "Google".
        let calendars = parse(SAMPLE).expect("parses");
        assert_eq!(calendars.len(), 3, "deleted entry should be dropped");
        assert_eq!(calendars[1].title, "Work");
        assert_eq!(calendars[1].color.as_deref(), Some("#f83a22"));
    }

    #[test]
    fn the_users_own_name_for_a_calendar_wins() {
        // Someone who renamed a calendar is looking for the name they gave it.
        let calendars = parse(SAMPLE).unwrap();
        assert_eq!(calendars[2].title, "My name for it");
    }

    #[test]
    fn the_account_is_read_from_the_primary_calendar() {
        // Its id is the account's email, so the UI can name the connected
        // account without asking for an identity scope on top.
        let calendars = parse(SAMPLE).unwrap();
        assert_eq!(
            account_email(&calendars).as_deref(),
            Some("chenwenlongofficial@gmail.com")
        );
    }

    #[test]
    fn no_primary_means_no_claim_about_the_account() {
        // Delegated and service accounts have no primary. Saying nothing beats
        // naming the wrong person as the connected account.
        let body = r#"{"items":[{"id":"a@group.calendar.google.com","summary":"A"}]}"#;
        assert_eq!(account_email(&parse(body).unwrap()), None);
    }

    #[test]
    fn a_primary_that_is_not_an_address_is_not_an_account() {
        let body = r#"{"items":[{"id":"not-an-email","summary":"X","primary":true}]}"#;
        assert_eq!(account_email(&parse(body).unwrap()), None);
    }

    #[test]
    fn an_empty_account_is_not_an_error() {
        assert!(parse(r#"{"items":[]}"#).unwrap().is_empty());
        assert!(parse(r#"{}"#).unwrap().is_empty());
    }

    #[test]
    fn a_calendar_with_no_summary_falls_back_to_its_id() {
        // Better than a blank row with a switch beside it.
        let body = r#"{"items":[{"id":"quiet@group.calendar.google.com"}]}"#;
        assert_eq!(
            parse(body).unwrap()[0].title,
            "quiet@group.calendar.google.com"
        );
    }

    #[test]
    fn source_ids_round_trip_and_stay_namespaced() {
        // These share a list with EventKit's calendars; an id collision would
        // silently toggle the wrong row.
        let id = source_id("abc@group.calendar.google.com");
        assert_eq!(id, "google:abc@group.calendar.google.com");
        assert_eq!(calendar_id_of(&id), Some("abc@group.calendar.google.com"));
        assert_eq!(
            calendar_id_of("work"),
            None,
            "an EventKit id is not Google's"
        );
    }
}
