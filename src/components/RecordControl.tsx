import { useCallback, useEffect, useRef, useState } from "react";
import {
  meetingStart,
  meetingState,
  meetingStop,
  onSidecarEvent,
  sidecarSend,
} from "../lib/tauri";
import type { MeetingState, SupervisorEvent } from "../types";

/**
 * Recording, as a control rather than a card.
 *
 * The harness gave recording an entire panel: three buttons, a status badge and
 * two level meters spanning the window — more visual weight than the notes it
 * exists to serve. Granola gives the same job a corner of the screen (see
 * docs/ui-teardown.md), and so does this: a level indicator you can see from
 * the side of your eye, and one button whose meaning never changes shape.
 *
 * The three-state engine underneath is untouched. Arm still fills the pre-roll,
 * start still opens a meeting, stop still asks the sidecar to finish. Only the
 * presentation changed, which is the whole point of the goal.
 */

export type ControlMode = "idle" | "armed" | "recording" | "processing";

export function controlMode(state: MeetingState): ControlMode {
  return state.state as ControlMode;
}

/**
 * Whether stop should be offered.
 *
 * Not during `processing`. The sidecar has been asked to finish and is still
 * writing the file; a second stop there is either a no-op or a way to truncate
 * the recording being saved.
 */
export function canStop(mode: ControlMode): boolean {
  return mode === "recording";
}

/** Whether starting a new recording is allowed. */
export function canStart(mode: ControlMode): boolean {
  // `processing` is deliberately excluded. The previous meeting's audio is
  // still being written, and starting another would compete for the device
  // while looking, to the user, like nothing was wrong.
  return mode === "idle" || mode === "armed";
}

export function modeLabel(mode: ControlMode): string {
  switch (mode) {
    case "idle":
      return "Record";
    case "armed":
      // Armed is not recording. Saying "Recording" here would claim the meeting
      // is being kept when only the rolling buffer is filling.
      return "Ready";
    case "recording":
      return "Recording";
    case "processing":
      return "Finishing…";
  }
}

/**
 * How many indicator bars to light for a level in 0..1.
 *
 * Speech sits low in that range, so the raw value would leave the meter dark
 * through an entire meeting and read as "not hearing you". The same ×4 scaling
 * the old meters used is kept, then clamped.
 */
export function litBars(level: number, bars = 4): number {
  if (!Number.isFinite(level) || level <= 0) return 0;
  return Math.min(bars, Math.max(1, Math.round(level * 4 * bars)));
}

export function RecordControl() {
  const [mode, setMode] = useState<ControlMode>("idle");
  const [levels, setLevels] = useState({ mic: 0, system: 0 });
  const [message, setMessage] = useState<string | null>(null);
  const unlisten = useRef<(() => void) | null>(null);

  const refresh = useCallback(async () => {
    try {
      setMode(controlMode(await meetingState()));
    } catch {
      // A lifecycle that cannot be read is not worth an error in the corner of
      // every screen; the buttons simply stay as they were.
    }
  }, []);

  useEffect(() => {
    void refresh();
    // The lifecycle changes from outside this component — detection can start a
    // meeting with nobody touching the button — so it is polled rather than
    // assumed to follow our own clicks.
    const timer = setInterval(() => void refresh(), 1000);
    return () => clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const stop = await onSidecarEvent((event: SupervisorEvent) => {
        if (event.kind !== "event") return;
        if (event.event.ev === "level") {
          setLevels({ mic: event.event.mic, system: event.event.system });
        }
      });
      if (cancelled) stop();
      else unlisten.current = stop;
    })();
    return () => {
      cancelled = true;
      unlisten.current?.();
    };
  }, []);

  async function begin() {
    setMessage(null);
    try {
      // Arm fills the pre-roll so the first seconds of speech are not lost;
      // start opens the meeting. Both, in that order, from one button.
      await sidecarSend({ cmd: "arm" });
      await meetingStart();
      await refresh();
    } catch (err) {
      setMessage(String(err));
    }
  }

  async function end() {
    setMessage(null);
    try {
      await meetingStop();
      await refresh();
    } catch (err) {
      setMessage(String(err));
    }
  }

  const peak = Math.max(levels.mic, levels.system);

  return (
    <div className="reccontrol" data-testid="record-control">
      <div className={`reccontrol-pill reccontrol-pill--${mode}`}>
        <span className="reccontrol-bars" aria-hidden="true">
          {[0, 1, 2, 3].map((i) => (
            <span
              key={i}
              className={
                i < litBars(peak)
                  ? "reccontrol-bar reccontrol-bar--lit"
                  : "reccontrol-bar"
              }
            />
          ))}
        </span>

        <span className="reccontrol-label">{modeLabel(mode)}</span>

        {canStop(mode) ? (
          <button
            className="reccontrol-stop"
            onClick={() => void end()}
            aria-label="stop recording"
          >
            <span className="reccontrol-square" />
          </button>
        ) : (
          <button
            className="reccontrol-start"
            onClick={() => void begin()}
            disabled={!canStart(mode)}
            aria-label="start recording"
          >
            <span className="reccontrol-dot" />
          </button>
        )}
      </div>
      {message && <p className="reccontrol-message">{message}</p>}
    </div>
  );
}
