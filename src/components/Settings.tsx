import { DetectionSettingsPanel } from "./DetectionSettings";
import { GoogleCalendarCard } from "./GoogleCalendarCard";
import { NotionCard } from "./NotionCard";
import { PermissionsCard } from "./PermissionsCard";
import { PrivacyCard } from "./PrivacyCard";
import { ProviderCard } from "./ProviderCard";
import { SettingsGroup, SettingsRow } from "./SettingsRow";
import { UpdateCard } from "./UpdateCard";

/**
 * Everything that is about the app rather than about a meeting.
 *
 * Modelled on Granola's Preferences screen: a section label, then one rounded
 * group, then rows inside it sharing a dashed hairline that begins after the
 * icon column. Every row is the same shape, which is what lets a long settings
 * page be scanned rather than read.
 *
 * Three things are deliberately absent — the sidecar panel, the Rust core check
 * and the data layer. One of them offers "Simulate crash" and another prints an
 * event log; those are diagnostics, they live on the workbench, and a
 * diagnostic is not a setting however tidily it is framed.
 *
 * The cards inside these rows are unchanged and still own their behaviour and
 * their tests. What they lose is their frame.
 */
export function Settings({ onBack }: { onBack: () => void }) {
  return (
    <div className="settings" data-testid="settings">
      <button className="document-back" onClick={onBack}>
        ‹ Meetings
      </button>
      <h1 className="library-title settings-title">Preferences</h1>

      <SettingsGroup label="Capture">
        <SettingsRow icon="microphone">
          <PermissionsCard />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup label="Detection">
        <SettingsRow icon="eye">
          <DetectionSettingsPanel />
        </SettingsRow>
        <SettingsRow icon="calendar">
          <GoogleCalendarCard />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup label="Models">
        <SettingsRow icon="sparkle">
          <ProviderCard />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup label="Data and sharing">
        <SettingsRow icon="share">
          <NotionCard />
        </SettingsRow>
        <SettingsRow icon="lock">
          <PrivacyCard />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup label="About">
        <SettingsRow icon="info">
          <UpdateCard />
        </SettingsRow>
      </SettingsGroup>
    </div>
  );
}
