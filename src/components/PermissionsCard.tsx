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
  toggleAction,
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
  const [asking, setAsking] = useState<PrivacyPane | null>(null);
  /* Panes macOS has already been asked about in this session. Screen Recording
     cannot say whether it was ever asked, so the app has to remember. */
  const [asked, setAsked] = useState<PrivacyPane[]>([]);
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

  /**
   * Asks macOS for one capability.
   *
   * Scoped to the pane, because requesting both fires two system dialogs one
   * after the other — right for a first-run "set me up", startling from a
   * button sitting on one row.
   */
  const ask = async (pane: PrivacyPane) => {
    setAsking(pane);
    try {
      await guard(() => sidecarSend({ cmd: "permissions", request: true, pane }));
      setAsked((was) => (was.includes(pane) ? was : [...was, pane]));
    } finally {
      setAsking(null);
    }
  };

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
                {/* A real switch, and honest in both directions: on asks
                    macOS, off opens the pane where a grant can be revoked —
                    no API lets an app take back its own. It is never flipped
                    optimistically; it shows what macOS last reported, so
                    cancelling the dialog leaves it where it was. */}
                <input
                  type="checkbox"
                  role="switch"
                  className="perm-switch"
                  aria-label={cap.title}
                  checked={cap.state === "granted"}
                  disabled={asking === cap.pane}
                  onChange={() =>
                    toggleAction(cap, asked.includes(cap.pane)) === "prompt"
                      ? void ask(cap.pane)
                      : void openPane(cap.pane)
                  }
                />
              </div>
              <p className="perm-reason">{cap.reason}</p>
              {/* The remedy stays as a sentence, without a second control.
                  The switch is the action now, and two ways to do one thing on
                  one row is how a user learns to trust neither. */}
              {cap.remedy && (
                <p className="perm-remedy">
                  {asking === cap.pane ? "Waiting for macOS…" : cap.remedy}
                </p>
              )}
            </div>
          ))}
        </div>
      )}

      {message && <p className="empty-note">{message}</p>}
    </section>
  );
}
