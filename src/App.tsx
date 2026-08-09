import { useState } from "react";
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
import { Library } from "./components/Library";

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

/**
 * Which screen is showing.
 *
 * `workbench` is the old build harness — every diagnostic card the app grew
 * while its pipeline was being proved. Phase 8 replaces it with `library` and
 * a meeting document, but the cards still hold the only access to settings,
 * providers, permissions and export until G33 rehomes them. Deleting them now
 * would remove working features to make the app look finished, so it stays
 * reachable and unglamorous until there is somewhere else for its contents.
 */
export type View = "library" | "workbench";

function App() {
  // Which window, decided before any state exists. Keeping this in its own
  // component means MainWindow's hooks are never behind a branch — the popup
  // is a different window, but React cannot tell that from an early return.
  if (isPopupWindow(window.location.search, currentLabel())) {
    return <DetectionPopup />;
  }
  return <MainWindow />;
}

function MainWindow() {
  const [view, setView] = useState<View>("library");

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

  if (view === "library") {
    return (
      <main className="app">
        <header className="library-head">
          <h1 className="library-title">Meetings</h1>
          <button className="link-button" onClick={() => setView("workbench")}>
            Workbench
          </button>
        </header>
        <Onboarding />
        <Library
          onOpen={(meetingId) => {
            // G31 builds the meeting document. Until then the workbench is
            // where a meeting can actually be read, so opening one goes there
            // and asks it to reveal the meeting rather than silently no-oping.
            setView("workbench");
            reveal(meetingId, 0);
          }}
        />
      </main>
    );
  }

  return (
    <main className="app">
      <header className="masthead">
        <h1>Oatmeal</h1>
        <span className="phase-tag">Workbench</span>
        <button className="link-button" onClick={() => setView("library")}>
          ← Meetings
        </button>
      </header>
      <p className="tagline">
        The build harness. Each card proves one piece of the pipeline works end to end
        &mdash; not that the code compiles, but that the behaviour is observable. Phase
        8 is moving what belongs to users out of here.
      </p>

      <Onboarding />
      <RecordCard />
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
