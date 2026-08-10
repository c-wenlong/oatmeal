//! The menu bar item.
//!
//! The manual path, and the only one that is always available: detection can be
//! off, the calendar can be disconnected, permissions can be half-granted, and
//! this still records. Everything in Phase 5 is a convenience layered on top of
//! it, so it comes first and depends on nothing.
//!
//! macOS convention drives the shape — the elapsed time lives in the menu bar
//! title while recording, because a recording indicator you have to open a menu
//! to see is not an indicator.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

/// How many past meetings the menu offers.
///
/// Short on purpose: this is a shortcut to "the one I just recorded", not a
/// second library window.
pub const RECENT_LIMIT: i64 = 5;

/// Menu item ids. Matched in [`action_for`], so they live in one place rather
/// than as string literals at both ends.
pub const ID_RECORD: &str = "record";
pub const ID_STOP: &str = "stop";
pub const ID_OPEN: &str = "open";
pub const ID_QUIT: &str = "quit";
pub const RECENT_PREFIX: &str = "recent:";

/// Elapsed time as it appears in the menu bar.
///
/// Minutes and seconds even past an hour: `72:14` is unambiguous at a glance and
/// narrower than `1:12:14`, and menu bar width is genuinely scarce.
pub fn format_elapsed(ms: i64) -> String {
    let total = (ms.max(0)) / 1000;
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// How far ahead the menu bar will name a meeting.
///
/// Eight hours. Beyond that it is not "upcoming", it is "today", and a menu bar
/// that permanently reads `Standup · 7h` is just a strip of noise.
pub const NEXT_UP_HORIZON_MS: i64 = 8 * 3600 * 1000;

/// "Standup · 12m" — the next meeting and how long until it starts.
///
/// `None` when there is nothing to say, which is what keeps an idle app from
/// occupying menu bar width it has not earned. An event already under way
/// returns `None` too: "in -3m" is not a thing, and the meeting is on screen.
pub fn next_up_label(title: &str, starts_at_ms: i64, now_ms: i64) -> Option<String> {
    let remaining = starts_at_ms - now_ms;
    if !(0..=NEXT_UP_HORIZON_MS).contains(&remaining) {
        return None;
    }
    let minutes = remaining / 60_000;
    let when = if minutes < 1 {
        "now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else {
        // "1h" rather than "1h 0m": the zero adds a character and no meaning.
        let (hours, rest) = (minutes / 60, minutes % 60);
        if rest == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {rest}m")
        }
    };
    // Short: this shares the menu bar with everything else the user runs.
    Some(format!("{} · {when}", menu_label(title, 18)))
}

/// The title shown beside the icon. Empty when idle with nothing coming up —
/// an idle app should not occupy menu bar width it does not need.
///
/// Recording wins over the next meeting. Both are true at once often enough,
/// and the elapsed timer is the one the user is actually watching.
pub fn tray_title(recording: bool, elapsed_ms: i64, next_up: Option<&str>) -> String {
    if recording {
        format_elapsed(elapsed_ms)
    } else {
        next_up.unwrap_or_default().to_string()
    }
}

/// The tooltip, which is where the state is spelled out in words.
pub fn tray_tooltip(recording: bool, processing: bool) -> String {
    if recording {
        "Oatmeal — recording".into()
    } else if processing {
        "Oatmeal — finishing up".into()
    } else {
        "Oatmeal — idle".into()
    }
}

/// A meeting as the menu shows it.
pub struct RecentEntry {
    pub id: String,
    pub label: String,
}

/// Truncates a title to something that fits a menu without wrapping.
pub fn menu_label(title: &str, max: usize) -> String {
    let trimmed = title.trim();
    let name = if trimmed.is_empty() {
        "Untitled meeting"
    } else {
        trimmed
    };
    if name.chars().count() <= max {
        return name.to_string();
    }
    let kept: String = name.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// Builds the menu for the current state.
///
/// Rebuilt rather than mutated on every change: the record/stop item swaps
/// entirely and the recent list changes length, and a menu assembled in one
/// place is far easier to reason about than one patched from three.
pub fn build_menu<R: Runtime>(
    app: &AppHandle<R>,
    recording: bool,
    recent: &[RecentEntry],
) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;

    if recording {
        menu.append(&MenuItem::with_id(
            app,
            ID_STOP,
            "Stop recording",
            true,
            None::<&str>,
        )?)?;
    } else {
        menu.append(&MenuItem::with_id(
            app,
            ID_RECORD,
            "Record now",
            true,
            None::<&str>,
        )?)?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;

    if recent.is_empty() {
        // A disabled row rather than an absent one: an empty menu reads as
        // broken, and "no meetings yet" is information.
        let empty = MenuItem::with_id(app, "recent-empty", "No meetings yet", false, None::<&str>)?;
        menu.append(&empty)?;
    } else {
        let submenu = Submenu::new(app, "Recent meetings", true)?;
        for entry in recent {
            submenu.append(&MenuItem::with_id(
                app,
                format!("{RECENT_PREFIX}{}", entry.id),
                &entry.label,
                true,
                None::<&str>,
            )?)?;
        }
        menu.append(&submenu)?;
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        ID_OPEN,
        "Open Oatmeal",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        ID_QUIT,
        "Quit Oatmeal",
        true,
        None::<&str>,
    )?)?;

    Ok(menu)
}

/// What a menu click means, decided without touching Tauri.
///
/// Split out so the routing is testable: the handler itself can only be
/// exercised with a running app, and "clicking a recent meeting opens that
/// meeting" is exactly the kind of thing that silently breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    StartRecording,
    StopRecording,
    OpenWindow,
    OpenMeeting(String),
    Quit,
    Ignore,
}

pub fn action_for(id: &str) -> TrayAction {
    match id {
        ID_RECORD => TrayAction::StartRecording,
        ID_STOP => TrayAction::StopRecording,
        ID_OPEN => TrayAction::OpenWindow,
        ID_QUIT => TrayAction::Quit,
        other => match other.strip_prefix(RECENT_PREFIX) {
            // An empty id would open "the meeting called nothing", so it is not
            // treated as a meeting at all.
            Some(id) if !id.is_empty() => TrayAction::OpenMeeting(id.to_string()),
            _ => TrayAction::Ignore,
        },
    }
}

/// Brings the main window to the front, creating nothing — the window always
/// exists, it may just be hidden or behind something.
pub fn focus_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// The menu bar glyph: a ring with a filled centre, drawn as a template.
///
/// Deliberately not the app icon. Menu bar icons are monochrome by convention,
/// and this one has to read at 22pt against both a light and a dark bar.
fn tray_icon() -> tauri::Result<tauri::image::Image<'static>> {
    tauri::image::Image::from_bytes(include_bytes!("../icons/tray@2x.png"))
}

/// Installs the tray icon. Failure is reported, never fatal: a missing menu bar
/// item is a degraded app, not a broken one.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, false, &[])?;
    let handle = app.clone();

    TrayIconBuilder::with_id("oatmeal")
        .icon(tray_icon()?)
        // A template icon is alpha-only: macOS recolours it for light and dark
        // menu bars, and inverts it while the menu is open. The app icon cannot
        // be used here — templating a full-colour image collapses it to a solid
        // blob, since every opaque pixel becomes ink.
        .icon_as_template(true)
        .tooltip(tray_tooltip(false, false))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |_app, event: MenuEvent| {
            crate::on_tray_action(&handle, action_for(event.id().as_ref()));
        })
        .build(app)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_is_minutes_and_seconds() {
        assert_eq!(format_elapsed(0), "00:00");
        assert_eq!(format_elapsed(9_000), "00:09");
        assert_eq!(format_elapsed(65_000), "01:05");
    }

    #[test]
    fn elapsed_keeps_counting_minutes_past_an_hour() {
        // `1:12:14` is wider than `72:14` and no clearer, and menu bar width is
        // genuinely scarce.
        assert_eq!(format_elapsed(4_334_000), "72:14");
    }

    #[test]
    fn a_negative_clock_does_not_render_nonsense() {
        // Clock adjustments during a meeting are rare but not impossible.
        assert_eq!(format_elapsed(-5_000), "00:00");
    }

    #[test]
    fn an_idle_tray_takes_no_menu_bar_width() {
        assert_eq!(tray_title(false, 60_000, None), "");
        assert_eq!(tray_title(true, 60_000, None), "01:00");
    }

    #[test]
    fn recording_wins_the_menu_bar_over_the_next_meeting() {
        // Both are true at once often enough. The elapsed timer is the one the
        // user is watching, and the menu bar has room for exactly one.
        assert_eq!(tray_title(true, 60_000, Some("Standup · 5m")), "01:00");
        assert_eq!(tray_title(false, 0, Some("Standup · 5m")), "Standup · 5m");
    }

    #[test]
    fn the_next_meeting_reads_as_a_countdown() {
        let now = 1_000_000_000;
        assert_eq!(
            next_up_label("Standup", now + 12 * 60_000, now).as_deref(),
            Some("Standup · 12m")
        );
        assert_eq!(
            next_up_label("Standup", now + 30_000, now).as_deref(),
            Some("Standup · now")
        );
        assert_eq!(
            next_up_label("Standup", now + 65 * 60_000, now).as_deref(),
            Some("Standup · 1h 5m")
        );
        // "1h 0m" spends a character to say nothing.
        assert_eq!(
            next_up_label("Standup", now + 120 * 60_000, now).as_deref(),
            Some("Standup · 2h")
        );
    }

    #[test]
    fn a_meeting_already_under_way_is_not_counted_down_to() {
        // "in -3m" is not a thing, and the meeting is already on screen.
        let now = 1_000_000_000;
        assert!(next_up_label("Standup", now - 1, now).is_none());
    }

    #[test]
    fn nothing_far_enough_ahead_leaves_the_menu_bar_alone() {
        // A strip permanently reading `Standup · 7h` is noise, not information.
        let now = 1_000_000_000;
        assert!(next_up_label("Standup", now + NEXT_UP_HORIZON_MS + 1, now).is_none());
        assert!(next_up_label("Standup", now + NEXT_UP_HORIZON_MS, now).is_some());
    }

    #[test]
    fn a_long_title_is_cut_before_it_reaches_the_menu_bar() {
        let now = 1_000_000_000;
        let label = next_up_label(
            "Quarterly planning review with the whole team",
            now + 60_000,
            now,
        )
        .unwrap();
        assert!(label.chars().count() <= 18 + " · 1m".chars().count());
        assert!(label.ends_with(" · 1m"));
    }

    #[test]
    fn the_tooltip_distinguishes_finishing_from_idle() {
        // Stopping is not instant — the sidecar has to finalise the file. An
        // app that says "idle" during that looks like it lost the recording.
        assert_eq!(tray_tooltip(true, false), "Oatmeal — recording");
        assert_eq!(tray_tooltip(false, true), "Oatmeal — finishing up");
        assert_eq!(tray_tooltip(false, false), "Oatmeal — idle");
    }

    #[test]
    fn menu_labels_are_truncated_with_an_ellipsis() {
        assert_eq!(menu_label("Standup", 20), "Standup");
        assert_eq!(
            menu_label("Quarterly planning review with the platform team", 20),
            "Quarterly planning…"
        );
    }

    #[test]
    fn an_untitled_meeting_still_gets_a_label() {
        assert_eq!(menu_label("", 20), "Untitled meeting");
        assert_eq!(menu_label("   ", 20), "Untitled meeting");
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // A byte-based cut would split a multi-byte character and panic.
        let label = menu_label("日本語のミーティングについて話しましょう", 10);
        assert!(label.chars().count() <= 10, "{label}");
        assert!(label.ends_with('…'));
    }

    #[test]
    fn menu_ids_route_to_the_right_action() {
        assert_eq!(action_for(ID_RECORD), TrayAction::StartRecording);
        assert_eq!(action_for(ID_STOP), TrayAction::StopRecording);
        assert_eq!(action_for(ID_OPEN), TrayAction::OpenWindow);
        assert_eq!(action_for(ID_QUIT), TrayAction::Quit);
    }

    #[test]
    fn a_recent_meeting_click_carries_its_id() {
        assert_eq!(
            action_for("recent:m123"),
            TrayAction::OpenMeeting("m123".into())
        );
    }

    #[test]
    fn unknown_and_malformed_ids_do_nothing() {
        // The disabled "No meetings yet" row still emits an event on some
        // platforms; it must not be read as a meeting.
        assert_eq!(action_for("recent-empty"), TrayAction::Ignore);
        assert_eq!(action_for("recent:"), TrayAction::Ignore);
        assert_eq!(action_for("something-else"), TrayAction::Ignore);
    }

    #[test]
    fn a_meeting_id_containing_the_separator_survives_round_tripping() {
        // Ids are opaque; one containing a colon must not be truncated.
        assert_eq!(
            action_for("recent:a:b:c"),
            TrayAction::OpenMeeting("a:b:c".into())
        );
    }
}
