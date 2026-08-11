import { useCallback, useEffect, useState } from "react";
import {
  calendarRefreshGoogle,
  calendarResetVisibility,
  calendarSetDisplay,
  calendarSetVisible,
  calendarAccess,
  calendarSources,
  detectionSettings,
} from "../lib/tauri";
import { SettingsGroup, SettingsIcon } from "./SettingsRow";
import type { CalendarAccess, CalendarSource, DetectionSettings } from "../types";

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
 * with the event window rather than on request.
 *
 * A connected Google account appears as one row rather than several. Its scope
 * (`calendar.events.readonly`) cannot enumerate calendars, so there is nothing
 * to expand it into — but leaving it out of a list headed "visible calendars"
 * made the one source the user explicitly connected the one source they could
 * not find.
 */

/**
 * What to say when the list is empty, which is never just "no calendars".
 *
 * Three different situations, and only the user can tell them apart — so the
 * note names the one thing that is actually true rather than guessing. The
 * last case matters most: this list is EventKit's, so someone whose calendars
 * live only in Google will see it empty forever and has no way to know why
 * unless it is said.
 */
export function emptyListNote(
  calendarEnabled: boolean,
  loaded: boolean,
  access: CalendarAccess | null,
  googleConnected = false,
): string {
  // Someone whose calendar lives in Google has no reason to grant macOS
  // anything, and telling them to is advice that leads nowhere. The list is
  // not empty for them anyway — their account is a row in it — so this only
  // fires if that row is somehow missing too.
  if (googleConnected) {
    return "Your Google account is connected. Turn it on above to use its events for meeting detection.";
  }
  if (!calendarEnabled) {
    // The honest reason. "No calendars" would read as "you have none".
    return "Calendar detection is off, so Oatmeal is not reading your calendars. Turn it on under Detection.";
  }
  if (!loaded) return "Reading your calendars…";
  if (!access) {
    // The sidecar has never reported. That is itself the answer, and it is a
    // different problem from a permission the user could grant.
    return "The sidecar has not reported yet, so nothing has been read. Check the sidecar log under About.";
  }
  if (!access.authorized) {
    return "macOS has not granted Oatmeal access to your calendars. Open System Settings › Privacy & Security › Calendars.";
  }
  return (
    "macOS granted access and Oatmeal read your calendars, but the Calendar app has " +
    "none. A connected Google account is a separate source and appears above once it " +
    "is connected."
  );
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
  const [access, setAccess] = useState<CalendarAccess | null>(null);
  const [account, setAccount] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [next, list, granted] = await Promise.all([
        detectionSettings(),
        calendarSources(),
        calendarAccess(),
      ]);
      setSettings(next);
      setSources(list);
      setAccess(granted);
    } catch (err) {
      setError(String(err));
    }

    /* The account's calendars, fetched after the screen is already showing.
       A failure here leaves the cached list rather than emptying it — the
       calendars did not stop existing because the network did. */
    try {
      const email = await calendarRefreshGoogle();
      setAccount(email);
      if (email) setSources(await calendarSources());
    } catch {
      /* keeps whatever was cached */
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
  const googleConnected = list.some((source) => source.id === "google:primary");

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
          <h2 className="settings-section">
            Visible calendars
            {/* Which account these belong to. Read from the primary calendar's
                id, which is the address — no identity scope needed for it. */}
            {account && <span className="settings-account"> · {account}</span>}
          </h2>
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
            {emptyListNote(
              settings.calendarEnabled,
              sources !== null,
              access,
              googleConnected,
            )}
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
