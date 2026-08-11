import { useCallback, useEffect, useState } from "react";
import { DetectionSettingsPanel } from "./DetectionSettings";
import { detectionSettings, gcalSettings } from "../lib/tauri";
import { CalendarPane } from "./CalendarPane";
import { GoogleCalendarCard } from "./GoogleCalendarCard";
import { GoogleSetupGuide } from "./GoogleSetupGuide";
import { NotionCard } from "./NotionCard";
import { PermissionsCard } from "./PermissionsCard";
import { PrivacyCard } from "./PrivacyCard";
import { ProviderCard } from "./ProviderCard";
import { SettingsDisclosure, SettingsGroup, SettingsRow } from "./SettingsRow";
import { PANES, SettingsNav, type PaneId } from "./SettingsNav";
import { SidecarLogCard } from "./SidecarLogCard";
import { UpdateCard } from "./UpdateCard";
import type { GcalSettings } from "../types";

/**
 * Everything that is about the app rather than about a meeting.
 *
 * Modelled on Granola's Preferences window: a sidebar of panes on the left, and
 * on the right a section label, then one rounded group, then rows inside it
 * sharing a dashed hairline that begins after the icon column. Every row is the
 * same shape, which is what lets a settings page be scanned rather than read.
 *
 * Three things are deliberately absent — the sidecar panel, the Rust core check
 * and the data layer. One of them offers "Simulate crash" and another prints an
 * event log; those are diagnostics, they live on the workbench, and a
 * diagnostic is not a setting however tidily it is framed.
 *
 * The cards inside these rows are unchanged and still own their behaviour and
 * their tests. What they lose is their frame.
 */
/** What the detection row says at a glance. */
export function detectionSummary(
  settings: { micEnabled: boolean; calendarEnabled: boolean } | null,
): string {
  if (!settings) return "—";
  const on = [
    settings.micEnabled ? "apps" : null,
    settings.calendarEnabled ? "calendar" : null,
  ].filter(Boolean);
  // "Off" rather than an empty string: a blank value reads as still loading.
  return on.length === 0 ? "Off" : on.join(" and ");
}

/** What the Google Calendar row says at a glance. */
export function googleSummary(
  settings: { connected: boolean; clientId: string | null } | null,
): string {
  if (!settings) return "—";
  if (settings.connected) return "Connected";
  // "Not set up" rather than "Not connected" when there is no credential yet:
  // the two need different things from the user.
  return settings.clientId ? "Not connected" : "Not set up";
}

/** The heading over a pane — its sidebar name, said once more in full size. */
export function paneTitle(id: PaneId): string {
  return PANES.find((pane) => pane.id === id)?.label ?? "Settings";
}

export function Settings({ onBack }: { onBack: () => void }) {
  const [detection, setDetection] = useState<{
    micEnabled: boolean;
    calendarEnabled: boolean;
  } | null>(null);
  const [gcal, setGcal] = useState<GcalSettings | null>(null);
  const [pane, setPane] = useState<PaneId>("capture");
  const [detail, setDetail] = useState<"detection" | "google" | "google-guide" | null>(
    null,
  );

  const refresh = useCallback(async () => {
    try {
      setDetection(await detectionSettings());
    } catch {
      /* leaves the summary as "—" rather than claiming a state */
    }
  }, []);

  /* Only the pane that shows the summary asks for it, and it asks again when
     its detail closes — the detail is where the summary gets changed. Fetching
     on mount would call into Tauri to answer a question the About pane never
     puts. */
  useEffect(() => {
    if (pane === "detection") void refresh();
    if (pane === "calendar") {
      // Same reason as the detection summary: the row states a fact, and the
      // detail behind it is where that fact gets changed.
      gcalSettings()
        .then(setGcal)
        .catch(() => setGcal(null));
    }
  }, [refresh, pane, detail]);

  /* Leaving the pane leaves its sub-screen with it. Otherwise Detection would
     still be showing its detail when you came back to it from Models, having
     never asked for it. */
  const select = (id: PaneId) => {
    setDetail(null);
    setPane(id);
  };

  return (
    <div className="settings-shell" data-testid="settings">
      <SettingsNav current={pane} onSelect={select} onBack={onBack} />
      <div className="settings">
        {detail === "google-guide" ? (
          /* Back to the Google screen, not out to Calendar: the guide is read
             while setting that screen up, and landing a level higher would
             lose the place. */
          <GoogleSetupGuide onBack={() => setDetail("google")} />
        ) : detail === "google" ? (
          <div data-testid="settings-google">
            <button className="document-back" onClick={() => setDetail(null)}>
              ‹ Calendar
            </button>
            <h1 className="library-title settings-title">Google Calendar</h1>
            <div className="settings-group">
              <SettingsRow icon="share">
                <GoogleCalendarCard onOpenGuide={() => setDetail("google-guide")} />
              </SettingsRow>
            </div>
          </div>
        ) : detail === "detection" ? (
          <div data-testid="settings-detection">
            <button className="document-back" onClick={() => setDetail(null)}>
              ‹ Detection
            </button>
            <h1 className="library-title settings-title">Meeting detection</h1>
            <div className="settings-group">
              <SettingsRow icon="eye">
                <DetectionSettingsPanel />
              </SettingsRow>
            </div>
          </div>
        ) : (
          <>
            <h1 className="library-title settings-title">{paneTitle(pane)}</h1>
            {pane === "capture" && (
              <SettingsGroup label="Audio and screen">
                <SettingsRow icon="microphone">
                  <PermissionsCard />
                </SettingsRow>
              </SettingsGroup>
            )}

            {pane === "detection" && (
              <SettingsGroup label="Automatic recording">
                <SettingsDisclosure
                  icon="eye"
                  title="Meeting detection"
                  subtitle="Notice a meeting starting and offer to record it"
                  value={detectionSummary(detection)}
                  onOpen={() => setDetail("detection")}
                />
              </SettingsGroup>
            )}

            {pane === "calendar" && (
              <>
                <CalendarPane />
                {/* Behind a disclosure, like meeting detection: inline it is a
                    two-field credential form with its own prose, and it would
                    be the loudest thing on a page of quiet rows — for the
                    exception case that most people never touch. */}
                <SettingsGroup label="Other accounts">
                  <SettingsDisclosure
                    icon="share"
                    title="Google account"
                    subtitle="Connect a calendar the macOS Calendar app does not sync"
                    value={googleSummary(gcal)}
                    onOpen={() => setDetail("google")}
                  />
                </SettingsGroup>
              </>
            )}

            {pane === "models" && (
              <SettingsGroup label="Notes">
                <SettingsRow icon="sparkle">
                  <ProviderCard />
                </SettingsRow>
              </SettingsGroup>
            )}

            {pane === "sharing" && (
              <SettingsGroup label="Export">
                <SettingsRow icon="share">
                  <NotionCard />
                </SettingsRow>
              </SettingsGroup>
            )}

            {pane === "recordings" && (
              <SettingsGroup label="Stored on this Mac">
                <SettingsRow icon="lock">
                  <PrivacyCard />
                </SettingsRow>
              </SettingsGroup>
            )}

            {pane === "about" && (
              <>
                <SettingsGroup label="Version">
                  <SettingsRow icon="info">
                    <UpdateCard />
                  </SettingsRow>
                </SettingsGroup>
                {/* A diagnostic, and framed as one. It earns a place here
                    rather than on the workbench because it is what someone
                    reaches for when a screen is empty and they cannot tell
                    whether that is the app's fault or a permission. */}
                <SettingsGroup label="Diagnostics">
                  <SettingsRow icon="eye">
                    <SidecarLogCard />
                  </SettingsRow>
                </SettingsGroup>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}
