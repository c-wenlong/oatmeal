import { useCallback, useEffect, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  meetingActive,
  meetingStart,
  meetingStop,
  meetingTranscript,
  meetingsList,
  onSidecarEvent,
  sidecarSend,
} from "../lib/tauri";
import type { AudioSource, MeetingSummary, SupervisorEvent, Utterance } from "../types";

interface Line {
  key: string;
  source: AudioSource;
  text: string;
  startMs: number;
  /** In-flight text, superseded by a later final. */
  partial: boolean;
}

/** `mic` is the user, `system` is everyone else — SPEC section 4. */
function speaker(source: AudioSource): string {
  return source === "mic" ? "You" : "Them";
}

function timecode(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

/**
 * The first genuinely useful surface: record, watch the transcript arrive
 * attributed, and find it still there after a restart.
 */
export function RecordCard() {
  const [active, setActive] = useState<string | null>(null);
  const [lines, setLines] = useState<Line[]>([]);
  const [meetings, setMeetings] = useState<MeetingSummary[]>([]);
  const [viewing, setViewing] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [modelState, setModelState] = useState<string | null>(null);
  const [levels, setLevels] = useState<{ mic: number; system: number }>({
    mic: 0,
    system: 0,
  });

  const unlisten = useRef<UnlistenFn | null>(null);
  const transcriptRef = useRef<HTMLDivElement | null>(null);

  const refreshMeetings = useCallback(async () => {
    try {
      setMeetings(await meetingsList());
    } catch (err: unknown) {
      setMessage(String(err));
    }
  }, []);

  // A recording survives a window reload, so recover the active meeting rather
  // than showing an idle card while the sidecar is still capturing. With
  // nothing recording, open the most recent meeting — relaunching into an empty
  // pane hides the fact that anything was ever saved.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const current = await meetingActive();
        if (cancelled) return;
        setActive(current);

        const stored = await meetingsList();
        if (cancelled) return;
        setMeetings(stored);

        // Prefer the newest meeting that actually has a transcript. An empty
        // or interrupted one is the least useful thing to reopen into, and it
        // hides the fact that anything was ever saved.
        const latest = stored.find((m) => m.utteranceCount > 0);
        if (!current && latest) {
          const utterances = await meetingTranscript(latest.id);
          if (cancelled) return;
          setViewing(latest.id);
          setLines(
            utterances.map((u) => ({
              key: `stored-${u.id}`,
              source: u.source,
              text: u.text,
              startMs: u.startMs,
              partial: false,
            })),
          );
        }
      } catch (err: unknown) {
        if (!cancelled) setMessage(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const handle = useCallback(
    (event: SupervisorEvent) => {
      if (event.kind !== "event") return;
      const inner = event.event;

      if (inner.ev === "level") {
        setLevels({ mic: inner.mic, system: inner.system });
        return;
      }
      if (inner.ev === "model") {
        setModelState(inner.state);
        return;
      }
      if (inner.ev === "partial") {
        // One in-flight line per source, replaced as it grows.
        setLines((previous) => [
          ...previous.filter((l) => !(l.partial && l.source === inner.source)),
          {
            key: `partial-${inner.source}`,
            source: inner.source,
            text: inner.text,
            startMs: inner.t0,
            partial: true,
          },
        ]);
        return;
      }
      if (inner.ev === "final") {
        setLines((previous) => [
          ...previous.filter((l) => !(l.partial && l.source === inner.source)),
          {
            key: `final-${inner.source}-${inner.t0}-${inner.t1}`,
            source: inner.source,
            text: inner.text,
            startMs: inner.t0,
            partial: false,
          },
        ]);
        return;
      }
      if (inner.ev === "stopped") {
        setActive(null);
        refreshMeetings();
      }
    },
    [refreshMeetings],
  );

  useEffect(() => {
    let cancelled = false;
    onSidecarEvent(handle).then((fn) => {
      if (cancelled) fn();
      else unlisten.current = fn;
    });
    return () => {
      cancelled = true;
      unlisten.current?.();
      unlisten.current = null;
    };
  }, [handle]);

  useEffect(() => {
    if (transcriptRef.current) {
      transcriptRef.current.scrollTop = transcriptRef.current.scrollHeight;
    }
  }, [lines]);

  async function guard(action: () => Promise<void>) {
    setMessage(null);
    try {
      await action();
    } catch (err: unknown) {
      setMessage(String(err));
    }
  }

  const arm = () => guard(() => sidecarSend({ cmd: "arm" }));

  const start = () =>
    guard(async () => {
      setLines([]);
      setViewing(null);
      setActive(await meetingStart());
    });

  const stop = () => guard(() => meetingStop());

  /** Loads a stored transcript — the proof that it survived a restart. */
  const open = (meetingId: string) =>
    guard(async () => {
      const utterances: Utterance[] = await meetingTranscript(meetingId);
      setViewing(meetingId);
      setLines(
        utterances.map((u) => ({
          key: `stored-${u.id}`,
          source: u.source,
          text: u.text,
          startMs: u.startMs,
          partial: false,
        })),
      );
    });

  return (
    <section className="card">
      <div className="card-head">
        <h2>Record</h2>
        {active ? (
          <span className="pill pill--err">recording</span>
        ) : (
          <span className="pill pill--pending">idle</span>
        )}
      </div>
      <p className="card-note">
        Captures both streams, transcribes on device, and writes settled lines straight
        into SQLite. <strong>You</strong> is your microphone; <strong>Them</strong> is
        everything the machine is playing.
      </p>

      <div className="row">
        <button onClick={arm} disabled={active !== null}>
          Arm
        </button>
        <button className="primary" onClick={start} disabled={active !== null}>
          Start recording
        </button>
        <button onClick={stop} disabled={active === null}>
          Stop
        </button>
        <span className="meter" title="input levels">
          <span className="meter-label">You</span>
          <span className="meter-track">
            <span
              className="meter-fill meter-fill--mic"
              style={{ width: `${Math.min(100, levels.mic * 400)}%` }}
            />
          </span>
          <span className="meter-label">Them</span>
          <span className="meter-track">
            <span
              className="meter-fill meter-fill--system"
              style={{ width: `${Math.min(100, levels.system * 400)}%` }}
            />
          </span>
        </span>
      </div>

      {modelState && (
        <p className="empty-note">
          Speech model: {modelState}
          {modelState === "downloading" && " — first run downloads it"}
        </p>
      )}
      {message && <p className="empty-note">{message}</p>}

      <div className="log" ref={transcriptRef} data-testid="transcript">
        {lines.map((line) => (
          <div className="log-line" key={line.key}>
            <span className="log-time">{timecode(line.startMs)}</span>
            <span className={`log-tag log-tag--${line.source}`}>
              {speaker(line.source)}
            </span>
            <span className={line.partial ? "log-text log-partial" : "log-text"}>
              {line.text}
            </span>
          </div>
        ))}
      </div>

      <div className="meetings">
        <div className="meetings-head">
          <span>Stored meetings</span>
          <button onClick={refreshMeetings}>Refresh</button>
        </div>
        {meetings.length === 0 && <p className="empty-note">Nothing recorded yet.</p>}
        {meetings.map((meeting) => (
          <button
            key={meeting.id}
            className={viewing === meeting.id ? "meeting meeting--open" : "meeting"}
            onClick={() => open(meeting.id)}
          >
            <span className="meeting-title">{meeting.title ?? meeting.id}</span>
            <span className="meeting-meta">
              {meeting.utteranceCount} utterance
              {meeting.utteranceCount === 1 ? "" : "s"} &middot; {meeting.status}
              {meeting.audioPath ? " · audio saved" : ""}
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}
