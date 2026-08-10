import { useCallback, useEffect, useState } from "react";
import {
  calendarResetVisibility,
  calendarSetDisplay,
  calendarSetVisible,
  calendarSources,
  detectionSettings,
} from "../lib/tauri";
import { SettingsGroup, SettingsIcon } from "./SettingsRow";
import type { CalendarSource, DetectionSettings } from "../types";

/**
 * The Calendar settings pane, after Granola's.
 *
 * Two groups: what to display, and which calendars to watch. Every switch here
 * is wired to something that actually changes behaviour — the display switches
 * feed the detection rule and the menu bar, and the calendar list is the filter
 * applied before any event can raise a popup. Nothing on this page is a
 * placeholder for a feature that does not exist yet.
 *
 * The calendars come from EventKit via the sidecar, which is why they arrive
 * with the event window rather than on request. The Google OAuth path is not
 * represented here: it reads `primary` and nothing else, and its scope
 * (`calendar.events.readonly`) cannot ask for a calendar list at all.
 */

/** What to say when the list is empty, which is not always the same thing. */
export function emptyListNote(
  calendarEnabled: boolean,
  loaded: boolean,
): string | null {
  if (!calendarEnabled) {
    // The honest reason. "No calendars" would read as "you have none".
    return "Calendar detection is off, so Oatmeal is not reading your calendars.";
  }
  if (!loaded) return "Reading your calendars…";
  return "No calendars yet. They appear once macOS grants access and the first read finishes.";
}

/** A calendar's dot. Falls back to the text colour rather than to nothing. */
export function dotColor(source: Pick<CalendarSource, "color">): string {
  return source.color ?? "currentColor";
}

function Switch({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <input
      type="checkbox"
      role="switch"
      aria-label={label}
      checked={checked}
      onChange={(e) => onChange(e.target.checked)}
    />
  );
}

export function CalendarPane() {
  const [settings, setSettings] = useState<DetectionSettings | null>(null);
  const [sources, setSources] = useState<CalendarSource[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [next, list] = await Promise.all([detectionSettings(), calendarSources()]);
      setSettings(next);
      setSources(list);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function setDisplay(
    key: "includeSoloEvents" | "showUpcomingInMenuBar",
    enabled: boolean,
  ) {
    // Written through and re-read rather than tracked locally: the switch
    // should reflect what was stored, not what was clicked.
    await calendarSetDisplay(key, enabled);
    await refresh();
  }

  if (error) return <p className="empty-note">{error}</p>;
  if (!settings) return <p className="empty-note">Loading…</p>;

  const list = sources ?? [];
  const anyHidden = list.some((source) => !source.visible);

  return (
    <>
      <SettingsGroup label="Display">
        <div className="settings-row">
          <SettingsIcon name="calendar" />
          <div className="settings-row-body">
            <span className="settings-disclosure-title">
              Show upcoming meetings in menu bar
            </span>
            <span className="settings-disclosure-sub">
              Display your next meeting and time until it starts in the macOS menu bar
            </span>
          </div>
          <Switch
            label="show upcoming meetings in menu bar"
            checked={settings.showUpcomingInMenuBar}
            onChange={(next) => void setDisplay("showUpcomingInMenuBar", next)}
          />
        </div>

        <div className="settings-row">
          <SettingsIcon name="eye" />
          <div className="settings-row-body">
            <span className="settings-disclosure-title">
              Show events with no participants
            </span>
            <span className="settings-disclosure-sub">
              Offer to record entries with no video link, no location and nobody else on
              them
            </span>
          </div>
          <Switch
            label="show events with no participants"
            checked={settings.includeSoloEvents}
            onChange={(next) => void setDisplay("includeSoloEvents", next)}
          />
        </div>
      </SettingsGroup>

      <section className="settings-block">
        <div className="settings-section-head">
          <h2 className="settings-section">Visible calendars</h2>
          {/* Only offered when it would do something. A Reset that is always
              there is a button the user has to think about every time. */}
          {anyHidden && (
            <button
              className="link-button settings-reset"
              onClick={() => void calendarResetVisibility().then(refresh)}
            >
              Reset
            </button>
          )}
        </div>

        {list.length === 0 ? (
          <p className="empty-note">
            {emptyListNote(settings.calendarEnabled, sources !== null)}
          </p>
        ) : (
          <div className="settings-group">
            {list.map((source) => (
              <div className="settings-row calendar-row" key={source.id}>
                <span
                  className="calendar-dot"
                  style={{ background: dotColor(source) }}
                  aria-hidden="true"
                />
                <div className="settings-row-body">
                  <span className="settings-disclosure-title">{source.title}</span>
                </div>
                <Switch
                  label={source.title}
                  checked={source.visible}
                  onChange={(next) =>
                    void calendarSetVisible(source.id, next).then(refresh)
                  }
                />
              </div>
            ))}
          </div>
        )}
      </section>
    </>
  );
}
