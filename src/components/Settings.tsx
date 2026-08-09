import { DbCard } from "./DbCard";
import { DetectionSettingsPanel } from "./DetectionSettings";
import { GoogleCalendarCard } from "./GoogleCalendarCard";
import { HealthCard } from "./HealthCard";
import { NotionCard } from "./NotionCard";
import { PermissionsCard } from "./PermissionsCard";
import { PrivacyCard } from "./PrivacyCard";
import { ProviderCard } from "./ProviderCard";
import { SidecarCard } from "./SidecarCard";
import { UpdateCard } from "./UpdateCard";

/**
 * Everything that is about the app rather than about a meeting.
 *
 * These are the same cards the harness showed on its front page: which model
 * generates panels, whether the microphone is permitted, how long audio is
 * kept, where exports go, whether an update is waiting. All of it is real and
 * none of it belongs in front of someone reading their notes — G33 in
 * docs/ui-teardown.md.
 *
 * They stay cards on purpose. A settings screen is exactly the place a bordered
 * panel per topic is the right shape; the teardown's objection was to cards in
 * the *document*, not to cards existing.
 */
export function Settings({ onBack }: { onBack: () => void }) {
  return (
    <div className="settings" data-testid="settings">
      <button className="document-back" onClick={onBack}>
        ‹ Meetings
      </button>
      <h1 className="library-title settings-title">Settings</h1>

      <h2 className="settings-section">Capture</h2>
      <PermissionsCard />
      <SidecarCard />

      <h2 className="settings-section">Detection</h2>
      <DetectionSettingsPanel />
      <GoogleCalendarCard />

      <h2 className="settings-section">Models</h2>
      <ProviderCard />

      <h2 className="settings-section">Export</h2>
      <NotionCard />

      <h2 className="settings-section">Privacy and data</h2>
      <PrivacyCard />
      <DbCard />

      <h2 className="settings-section">About</h2>
      <UpdateCard />
      <HealthCard />
    </div>
  );
}
