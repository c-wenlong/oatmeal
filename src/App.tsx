import { useState } from "react";
import { RecordCard } from "./components/RecordCard";
import { DetectionPopup } from "./components/DetectionPopup";
import { SearchCard } from "./components/SearchCard";
import { ChatCard } from "./components/ChatCard";
import { Onboarding } from "./components/Onboarding";
import { Library } from "./components/Library";
import { MeetingDocument } from "./components/MeetingDocument";
import { RecordControl } from "./components/RecordControl";
import { Settings } from "./components/Settings";
import { OverflowMenu } from "./components/OverflowMenu";

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
export type View =
  | { screen: "library" }
  | { screen: "meeting"; meetingId: string }
  | { screen: "settings" }
  | { screen: "workbench" };

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
  const [view, setView] = useState<View>({ screen: "library" });

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

  if (view.screen === "meeting") {
    return (
      <main className="app">
        <MeetingDocument
          meetingId={view.meetingId}
          onBack={() => setView({ screen: "library" })}
        />
        {/* Fixed to the window, not the document: the one control that must
            never be scrolled away from. Absent on the workbench, which still
            has the old card until G33 takes it apart — two recording UIs on
            one screen would be worse than either. */}
        <RecordControl />
      </main>
    );
  }

  if (view.screen === "settings") {
    return (
      <main className="app">
        <Settings onBack={() => setView({ screen: "library" })} />
      </main>
    );
  }

  if (view.screen === "library") {
    return (
      <main className="app">
        <header className="library-head">
          <h1 className="library-title">Meetings</h1>
          <OverflowMenu
            items={[
              { label: "Settings", onSelect: () => setView({ screen: "settings" }) },
              // The workbench survives here alone. It still holds the transcript
              // and the sidecar log, and G35 is what finally gives those a home.
              { label: "Workbench", onSelect: () => setView({ screen: "workbench" }) },
            ]}
          />
        </header>
        <Onboarding />
        <Library onOpen={(meetingId) => setView({ screen: "meeting", meetingId })} />
        <RecordControl />
      </main>
    );
  }

  return (
    <main className="app">
      <header className="masthead">
        <h1>Oatmeal</h1>
        <span className="phase-tag">Workbench</span>
        <button className="link-button" onClick={() => setView({ screen: "library" })}>
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
    </main>
  );
}

export default App;
