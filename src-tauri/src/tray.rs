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

/// The title shown beside the icon. Empty when idle — an idle app should not
/// occupy menu bar width it does not need.
pub fn tray_title(recording: bool, elapsed_ms: i64) -> String {
    if recording {
        format_elapsed(elapsed_ms)
    } else {
        String::new()
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
        assert_eq!(tray_title(false, 60_000), "");
        assert_eq!(tray_title(true, 60_000), "01:00");
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
