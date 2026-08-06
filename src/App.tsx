import { DbCard } from "./components/DbCard";
import { HealthCard } from "./components/HealthCard";
import { RecordCard } from "./components/RecordCard";
import { ProviderCard } from "./components/ProviderCard";
import { PermissionsCard } from "./components/PermissionsCard";
import { SidecarCard } from "./components/SidecarCard";
import { DetectionPopup } from "./components/DetectionPopup";
import { DetectionSettingsPanel } from "./components/DetectionSettings";
import { SearchCard } from "./components/SearchCard";
import { ChatCard } from "./components/ChatCard";
import { PrivacyCard } from "./components/PrivacyCard";
import { GoogleCalendarCard } from "./components/GoogleCalendarCard";
import { UpdateCard } from "./components/UpdateCard";
import { NotionCard } from "./components/NotionCard";
import { Onboarding } from "./components/Onboarding";

/**
 * Which window this is.
 *
 * The popup is a second Tauri window loading the same bundle, so the two are
 * told apart by label rather than by route — Tauri assigns the label at
 * creation, and it survives a reload that a hash route would not.
 */
export function isPopupWindow(search: string, label?: string): boolean {
  if (label) return label === "popup";
  return new URLSearchParams(search).get("window") === "popup";
}

function currentLabel(): string | undefined {
  // Present only inside a Tauri webview; absent in tests and in a browser.
  return (
    globalThis as {
      __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
    }
  ).__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
}

function App() {
  if (isPopupWindow(window.location.search, currentLabel())) {
    return <DetectionPopup />;
  }

  /**
   * Opens a meeting and scrolls to a moment in it.
   *
   * Routed through an event rather than lifted state: `RecordCard` already
   * owns which meeting is open and how to reveal a line, and duplicating that
   * here would give two components an opinion about the same thing.
   */
  const reveal = (meetingId: string, utteranceId: number) => {
    window.dispatchEvent(
      new CustomEvent("oatmeal:reveal", { detail: { meetingId, utteranceId } }),
    );
  };

  return (
    <main className="app">
      <header className="masthead">
        <h1>Oatmeal</h1>
        <span className="phase-tag">Phase 5</span>
      </header>
      <p className="tagline">
        Build harness. Each card proves one piece of the pipeline works end to end
        &mdash; not that the code compiles, but that the behaviour is observable.
      </p>

      {/* Recording first — it is what the app is for. Then the pre-flight that
          gates it, then the diagnostics. The sidecar log grows without bound, so
          it sits below anything that needs to stay glanceable. */}
      {/* First run, above everything: someone who cannot record yet has no use
          for the cards below it. Renders nothing once setup is done. */}
      <Onboarding />
      <RecordCard />
      {/* Search and Ask sit directly under the recording surface: they are what
          the corpus is *for*, and burying them under diagnostics would say the
          opposite. */}
      <SearchCard onReveal={reveal} />
      <ChatCard onReveal={reveal} />
      <DetectionSettingsPanel />
      <GoogleCalendarCard />
      <UpdateCard />
      <NotionCard />
      <PrivacyCard />
      <PermissionsCard />
      <ProviderCard />
      <HealthCard />
      <SidecarCard />
      <DbCard />
    </main>
  );
}

export default App;
