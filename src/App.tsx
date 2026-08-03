import { DbCard } from "./components/DbCard";
import { HealthCard } from "./components/HealthCard";
import { RecordCard } from "./components/RecordCard";
import { ProviderCard } from "./components/ProviderCard";
import { PermissionsCard } from "./components/PermissionsCard";
import { SidecarCard } from "./components/SidecarCard";
import { DetectionPopup } from "./components/DetectionPopup";
import { DetectionSettingsPanel } from "./components/DetectionSettings";

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
      <RecordCard />
      <DetectionSettingsPanel />
      <PermissionsCard />
      <ProviderCard />
      <HealthCard />
      <SidecarCard />
      <DbCard />
    </main>
  );
}

export default App;
