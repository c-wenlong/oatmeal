import { useCallback, useEffect, useState } from "react";
import {
  detectionBuiltinApps,
  detectionRuleClear,
  detectionRulesList,
  detectionSetSettings,
  detectionSettings,
} from "../lib/tauri";
import type { DetectionRule, DetectionSettings as Settings } from "../types";

/** Seconds, as a person would say them. */
export function formatLead(ms: number): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return rest === 0 ? `${minutes} min` : `${minutes} min ${rest}s`;
}

/**
 * Which apps may offer, and how far ahead the calendar fires.
 *
 * The two lists are the point. "Nothing fires without a rule" is only a
 * trustworthy promise if the user can see every rule and change any of them —
 * otherwise it is just an assertion in a changelog.
 */
export function DetectionSettingsPanel() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [rules, setRules] = useState<DetectionRule[]>([]);
  const [builtins, setBuiltins] = useState<[string, string][]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSettings(await detectionSettings());
      setRules(await detectionRulesList());
      setBuiltins(await detectionBuiltinApps());
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function save(next: Settings) {
    setSettings(next);
    setError(null);
    try {
      await detectionSetSettings(next);
    } catch (err) {
      setError(String(err));
    }
  }

  async function clearRule(bundleId: string) {
    await detectionRuleClear(bundleId);
    await refresh();
  }

  if (!settings) {
    return <p className="empty-note">{error ?? "Loading…"}</p>;
  }

  const ruled = new Set(rules.map((r) => r.bundleId));
  // A built-in that the user has since ruled on is shown in their list, not
  // here — otherwise the same app appears twice saying opposite things.
  const defaultsStillActive = builtins.filter(([id]) => !ruled.has(id));

  return (
    <section className="card" data-testid="detection-settings">
      <div className="card-head">
        <h2>Meeting detection</h2>
      </div>
      <p className="card-note">
        Oatmeal can notice when a meeting is starting and offer to record it.{" "}
        <strong>It never starts on its own</strong> — every recording needs a click.
      </p>

      <label className="tuner-row">
        <span>Watch for calls in other apps</span>
        <input
          type="checkbox"
          checked={settings.micEnabled}
          aria-label="watch for calls in other apps"
          onChange={(e) => void save({ ...settings, micEnabled: e.target.checked })}
        />
      </label>

      <label className="tuner-row">
        <span>Use my calendar</span>
        <input
          type="checkbox"
          checked={settings.calendarEnabled}
          aria-label="use my calendar"
          onChange={(e) =>
            void save({ ...settings, calendarEnabled: e.target.checked })
          }
        />
      </label>

      <label className="tuner-row">
        <span>Offer {formatLead(settings.leadMs)} before a calendar event</span>
        <input
          type="range"
          min={0}
          max={600_000}
          step={30_000}
          value={settings.leadMs}
          aria-label="calendar lead time"
          disabled={!settings.calendarEnabled}
          onChange={(e) => void save({ ...settings, leadMs: Number(e.target.value) })}
        />
      </label>

      <div className="rule-columns">
        <div>
          <h3>Allowed</h3>
          <ul className="rule-list" data-testid="allowed-apps">
            {rules
              .filter((r) => r.mode === "allow")
              .map((rule) => (
                <li key={rule.bundleId}>
                  <span>{rule.appName ?? rule.bundleId}</span>
                  <button
                    className="link-button"
                    onClick={() => void clearRule(rule.bundleId)}
                  >
                    Reset
                  </button>
                </li>
              ))}
            {defaultsStillActive.map(([id, name]) => (
              <li key={id}>
                <span>
                  {name} <span className="empty-note">· default</span>
                </span>
              </li>
            ))}
          </ul>
        </div>

        <div>
          <h3>Ignored</h3>
          <ul className="rule-list" data-testid="ignored-apps">
            {rules
              .filter((r) => r.mode === "ignore")
              .map((rule) => (
                <li key={rule.bundleId}>
                  <span>{rule.appName ?? rule.bundleId}</span>
                  <button
                    className="link-button"
                    onClick={() => void clearRule(rule.bundleId)}
                  >
                    Reset
                  </button>
                </li>
              ))}
            {rules.filter((r) => r.mode === "ignore").length === 0 && (
              <li className="empty-note">
                Nothing ignored. Apps you say “never” to appear here.
              </li>
            )}
          </ul>
        </div>
      </div>

      <p className="empty-note">
        An app that is not listed is asked about once, the first time it uses your
        microphone.
      </p>
      {error && <p className="empty-note">{error}</p>}
    </section>
  );
}
