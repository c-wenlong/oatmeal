import { DetectionSettingsPanel } from "./DetectionSettings";
import { GoogleCalendarCard } from "./GoogleCalendarCard";
import { NotionCard } from "./NotionCard";
import { PermissionsCard } from "./PermissionsCard";
import { PrivacyCard } from "./PrivacyCard";
import { ProviderCard } from "./ProviderCard";
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
 * Three things that were here have gone back to the workbench: the sidecar
 * panel, the Rust core check and the data layer. Those are diagnostics — one of
 * them offers a "Simulate crash" button and prints an event log — and a
 * diagnostic is not a setting no matter how tidily it is framed.
 *
 * What remains is styled as rows rather than cards. G33 argued a settings screen
 * was the right place for a bordered panel per topic; seen beside the library
 * and the document, it plainly was not. The panels made settings the one screen
 * that still looked like the harness.
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

      <h2 className="settings-section">Detection</h2>
      <DetectionSettingsPanel />
      <GoogleCalendarCard />

      <h2 className="settings-section">Models</h2>
      <ProviderCard />

      <h2 className="settings-section">Export</h2>
      <NotionCard />

      <h2 className="settings-section">Privacy and data</h2>
      <PrivacyCard />

      <h2 className="settings-section">About</h2>
      <UpdateCard />
    </div>
  );
}
