import { useCallback, useEffect, useState } from "react";
import { privacyPurgeAudio, privacySetRetention, privacySnapshot } from "../lib/tauri";
import { formatBytes } from "./ModelPicker";
import type { PrivacySnapshot, Provenance, Retention } from "../types";

/**
 * Whether a generation stayed on the machine.
 *
 * Reads the verdict Rust computed rather than matching the provider string
 * here. `panels.provider` stores a *display label* for older rows, and an
 * earlier version of this function compared it against snake_case enum names —
 * so every local summary was reported as cloud, in the one place that must not
 * be wrong about it.
 */
export function isLocal(entry: Pick<Provenance, "local">): boolean {
  return entry.local;
}

/** How a generation's provenance reads in the panel. */
export function provenanceLabel(entry: Provenance): string {
  const provider = entry.provider ?? "unknown";
  const model = entry.model ? ` · ${entry.model}` : "";
  return `${provider}${model}`;
}

export function retentionLabel(retention: Retention): string {
  if (retention.kind === "forever") return "kept forever";
  return retention.days === 1 ? "1 day" : `${retention.days} days`;
}

const CHOICES: Retention[] = [
  { kind: "days", days: 1 },
  { kind: "days", days: 7 },
  { kind: "days", days: 30 },
  { kind: "days", days: 90 },
  { kind: "forever" },
];

function sameRetention(a: Retention, b: Retention): boolean {
  if (a.kind === "forever" || b.kind === "forever") return a.kind === b.kind;
  return a.days === b.days;
}

/**
 * What is on disk, and what has left the machine.
 *
 * The provenance list is per *generation*, not per app, because that is the
 * only honest answer: someone who tried a cloud model once and then switched to
 * local needs to know which summaries went out, and a current-provider setting
 * cannot tell them.
 */
export function PrivacyCard() {
  const [snapshot, setSnapshot] = useState<PrivacySnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSnapshot(await privacySnapshot());
    } catch (err) {
      setMessage(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function choose(retention: Retention) {
    await privacySetRetention(retention);
    await refresh();
  }

  async function purge() {
    if (!snapshot) return;
    // Spelled out because "delete all audio" reads like it might take the
    // meetings too. It does not.
    if (
      !window.confirm(
        `Delete all ${snapshot.audioFiles} audio file(s)? Transcripts, notes and summaries are kept.`,
      )
    ) {
      return;
    }
    setBusy(true);
    try {
      const report = await privacyPurgeAudio();
      setMessage(
        `Deleted ${report.deleted} file(s), freeing ${formatBytes(report.freedBytes)}. Transcripts untouched.`,
      );
      await refresh();
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  if (!snapshot) {
    return (
      <section className="card">
        <div className="card-head">
          <h2>Privacy</h2>
        </div>
        <p className="empty-note">{message ?? "Loading…"}</p>
      </section>
    );
  }

  const cloud = snapshot.generations.filter((entry) => !isLocal(entry));

  return (
    <section className="card" data-testid="privacy-card">
      <div className="card-head">
        <h2>Privacy</h2>
        {!snapshot.telemetry && <span className="pill pill--ok">no telemetry</span>}
      </div>
      <p className="card-note">
        Recording and transcription happen entirely on this Mac. The only thing that can
        leave is a summary, and only if you pick a cloud provider.
      </p>

      <div className="row folder-bar">
        <span className="empty-note">Keep audio for</span>
        {CHOICES.map((choice) => (
          <button
            key={retentionLabel(choice)}
            className={
              sameRetention(choice, snapshot.retention) ? "chip chip--on" : "chip"
            }
            onClick={() => void choose(choice)}
          >
            {retentionLabel(choice)}
          </button>
        ))}
      </div>

      <p className="empty-note">
        {snapshot.audioFiles} audio file{snapshot.audioFiles === 1 ? "" : "s"} on disk,{" "}
        {formatBytes(snapshot.audioBytes)}. Expired audio is deleted when Oatmeal
        starts.
      </p>

      <div className="row">
        <button
          onClick={() => void purge()}
          disabled={busy || snapshot.audioFiles === 0}
        >
          {busy ? "Deleting…" : "Delete all audio now"}
        </button>
      </div>

      <h3 className="privacy-heading">Where your summaries were generated</h3>
      {snapshot.generations.length === 0 && (
        <p className="empty-note">Nothing generated yet.</p>
      )}
      {cloud.length > 0 && (
        <p className="empty-note">
          <strong>
            {cloud.length} summar{cloud.length === 1 ? "y" : "ies"} used a cloud
            provider.
          </strong>{" "}
          The transcript was sent to generate them.
        </p>
      )}
      <ul className="rule-list" data-testid="provenance">
        {snapshot.generations.map((entry) => (
          <li key={entry.panelId}>
            <span>{entry.meetingTitle?.trim() || "Untitled meeting"}</span>
            <span className={isLocal(entry) ? "pill pill--ok" : "pill pill--err"}>
              {isLocal(entry) ? "on device" : "cloud"} · {provenanceLabel(entry)}
            </span>
          </li>
        ))}
      </ul>

      {message && <p className="empty-note">{message}</p>}
    </section>
  );
}
