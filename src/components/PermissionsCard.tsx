import { useEffect, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  onSidecarEvent,
  openPrivacySettings,
  permissionsSnapshot,
  sidecarSend,
} from "../lib/tauri";
import {
  blocksCapture,
  canPrompt,
  capabilities,
  headline,
  type PermissionsSnapshot,
} from "../lib/permissions";
import type { PrivacyPane } from "../types";

/**
 * The tone of the card's own indicator.
 *
 * Three states, because "the sidecar has not reported yet" is not the same as
 * "capture is blocked" — one is unknown and the other is a problem the user
 * has to fix.
 */
export function headlineTone(
  snapshot: PermissionsSnapshot | null,
  ready: boolean,
): "ok" | "pending" | "err" {
  if (snapshot === null) return "pending";
  return ready ? "ok" : "err";
}

/**
 * Pre-flight for capture. Recording without both permissions produces a silent
 * or half-empty transcript, so this has to be checked before G6 ever starts a
 * stream — not discovered afterwards from an empty file.
 */
export function PermissionsCard() {
  const [snapshot, setSnapshot] = useState<PermissionsSnapshot | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const unlisten = useRef<UnlistenFn | null>(null);

  // Seed from the cached snapshot. Without this the card shows "unknown"
  // forever whenever it mounts after the sidecar already reported — which is
  // the normal case on a hot reload or a second window.
  useEffect(() => {
    let cancelled = false;
    permissionsSnapshot()
      .then((cached) => {
        if (!cancelled && cached) setSnapshot(cached);
      })
      .catch(() => {
        /* no cache yet is not an error worth showing */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    onSidecarEvent((event) => {
      if (event.kind === "event" && event.event.ev === "permissions") {
        setSnapshot({
          microphone: event.event.microphone,
          screenRecording: event.event.screen_recording,
          needsRelaunch: event.event.needs_relaunch,
        });
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten.current = fn;
    });
    return () => {
      cancelled = true;
      unlisten.current?.();
      unlisten.current = null;
    };
  }, []);

  async function guard(action: () => Promise<void>) {
    setMessage(null);
    try {
      await action();
    } catch (err: unknown) {
      setMessage(String(err));
    }
  }

  const check = (request: boolean) =>
    guard(() => sidecarSend({ cmd: "permissions", request }));

  const openPane = (pane: PrivacyPane) => guard(() => openPrivacySettings(pane));

  const ready = snapshot !== null && !blocksCapture(snapshot);

  return (
    <section className="card">
      <div className="card-head">
        <h2>Permissions</h2>
        {/* No badge. The sentence below already says whether capture can
            happen, and READY next to "Ready to record." is the same fact
            twice in two typefaces. */}
        <span
          className={`perm-dot perm-dot--${headlineTone(snapshot, ready)}`}
          aria-hidden="true"
        />
      </div>
      <p className="card-note">
        {snapshot === null
          ? "Start the sidecar, then check permissions. Both are required before any audio can be captured."
          : headline(snapshot)}
      </p>

      <div className="row">
        <button className="primary" onClick={() => check(false)}>
          Check permissions
        </button>
        <button
          onClick={() => check(true)}
          disabled={snapshot === null || !canPrompt(snapshot)}
          title={
            snapshot !== null && !canPrompt(snapshot)
              ? "macOS only shows the prompt once; use System Settings instead"
              : undefined
          }
        >
          Request access
        </button>
      </div>

      {snapshot !== null && (
        <div style={{ marginTop: 16 }}>
          {capabilities(snapshot).map((cap) => (
            <div className="perm" key={cap.pane}>
              <div className="perm-head">
                <span className="perm-title">{cap.title}</span>
                {/* The state, said once, where a switch would be. `title` so
                    the word is still reachable, and an accessible label so it
                    is not colour alone. */}
                <span
                  className={`perm-dot perm-dot--${cap.tone}`}
                  role="img"
                  aria-label={`${cap.title}: ${cap.state}`}
                  title={cap.state}
                />
              </div>
              <p className="perm-reason">{cap.reason}</p>
              {cap.remedy && (
                <div className="row">
                  <p className="perm-remedy">{cap.remedy}</p>
                  <button onClick={() => openPane(cap.pane)}>Open Settings</button>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {message && <p className="empty-note">{message}</p>}
    </section>
  );
}
