import { DbCard } from "./components/DbCard";
import { HealthCard } from "./components/HealthCard";
import { RecordCard } from "./components/RecordCard";
import { ProviderCard } from "./components/ProviderCard";
import { PermissionsCard } from "./components/PermissionsCard";
import { SidecarCard } from "./components/SidecarCard";

function App() {
  return (
    <main className="app">
      <header className="masthead">
        <h1>Oatmeal</h1>
        <span className="phase-tag">Phase 4</span>
      </header>
      <p className="tagline">
        Build harness. Each card proves one piece of the pipeline works end to end
        &mdash; not that the code compiles, but that the behaviour is observable.
      </p>

      {/* Recording first — it is what the app is for. Then the pre-flight that
          gates it, then the diagnostics. The sidecar log grows without bound, so
          it sits below anything that needs to stay glanceable. */}
      <RecordCard />
      <PermissionsCard />
      <ProviderCard />
      <HealthCard />
      <SidecarCard />
      <DbCard />
    </main>
  );
}

export default App;
