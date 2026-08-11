import { useEffect, useRef, useState } from "react";
import { onSidecarEvent } from "../lib/tauri";
import type { ModelState, UnlistenLike } from "../types";

/**
 * What the recorder is doing, said while it does it.
 *
 * Recording used to show the word "recording" and nothing else. On a cold
 * start the speech model takes about twelve seconds to load, during which no
 * text appears — so the honest reading of the screen was that it was broken,
 * and the only way to find out otherwise was to wait a minute and check the
 * database afterwards. Measured on this machine: a 53-second recording
 * produced its first line at the 25-second mark.
 */

export interface LiveState {
  /** Where the speech model is. Null until it says. */
  model: ModelState | null;
  /** The most recent in-flight text. Superseded constantly; never stored. */
  partial: string;
  /** How many settled lines this session has produced. */
  finals: number;
}

/**
 * One line saying where things stand.
 *
 * Ordered by what the user most needs to know: a model that is still loading
 * explains the silence, and nothing else does.
 */
export function liveStatus(state: LiveState, recording: boolean): string {
  if (!recording) return "";
  if (state.model === "downloading") return "Downloading the speech model…";
  if (state.model === "loading" || state.model === null) {
    // The twelve seconds that used to look like a hang.
    return "Loading the speech model — this takes a few seconds the first time";
  }
  if (state.model === "failed") return "The speech model failed to load.";
  if (state.partial) return state.partial;
  if (state.finals > 0) return `Listening · ${state.finals} lines so far`;
  return "Listening…";
}

/** Whether the status is text the model produced, rather than a state note. */
export function isTranscript(state: LiveState, recording: boolean): boolean {
  return recording && state.model === "ready" && state.partial.length > 0;
}

export function LiveTranscript({ recording }: { recording: boolean }) {
  const [state, setState] = useState<LiveState>({
    model: null,
    partial: "",
    finals: 0,
  });
  const unlisten = useRef<UnlistenLike | null>(null);

  useEffect(() => {
    let cancelled = false;
    onSidecarEvent((event) => {
      if (event.kind !== "event") return;
      const inner = event.event;
      if (inner.ev === "model") {
        setState((was) => ({ ...was, model: inner.state }));
      } else if (inner.ev === "partial") {
        setState((was) => ({ ...was, partial: inner.text }));
      } else if (inner.ev === "final") {
        // The partial is cleared, not kept: it was this line in progress, and
        // leaving it would show the same words twice.
        setState((was) => ({ ...was, partial: "", finals: was.finals + 1 }));
      }
    }).then((off) => {
      if (cancelled) off();
      else unlisten.current = off;
    });
    return () => {
      cancelled = true;
      unlisten.current?.();
    };
  }, []);

  if (!recording) return null;

  return (
    <p
      className={isTranscript(state, recording) ? "live live--text" : "live"}
      data-testid="live-status"
      aria-live="polite"
    >
      {liveStatus(state, recording)}
    </p>
  );
}
