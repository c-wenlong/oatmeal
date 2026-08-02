import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  meetingDelete,
  meetingRename,
  meetingState,
  onMeetingState,
  meetingStart,
  meetingStop,
  meetingTranscript,
  meetingsList,
  onSidecarEvent,
  sidecarSend,
  meetingLinks,
  onMeetingIndexed,
} from "../lib/tauri";
import {
  buildLinkIndex,
  detailKey,
  explainLink,
  highlightedNotes,
  highlightedUtterances,
} from "../lib/links";
import type {
  AudioSource,
  MeetingState as MeetingStateT,
  MeetingSummary,
  StoredLink,
  SupervisorEvent,
  Utterance,
} from "../types";
import { Notepad } from "./Notepad";
import { PanelView } from "./PanelView";
import { LinkTuner } from "./LinkTuner";

interface Line {
  key: string;
  /** Present on stored lines so a citation chip can find them. */
  utteranceId?: number;
  source: AudioSource;
  text: string;
  startMs: number;
  /** In-flight text, superseded by a later final. */
  partial: boolean;
}

/**
 * A transcript line's class.
 *
 * Two separate highlights land on the same element: the flash from clicking a
 * citation, and the steady glow from hovering a linked note. They are different
 * states — one is a one-shot "look here", the other says "this is where that
 * note came from" — so they get different classes rather than one being made to
 * stand in for the other.
 */
export function transcriptLineClass(
  utteranceId: number | undefined,
  flashed: number | null,
  lit: Set<number>,
): string {
  const classes = ["log-line"];
  if (utteranceId !== undefined && utteranceId === flashed) {
    classes.push("transcript-line--highlight");
  }
  if (utteranceId !== undefined && lit.has(utteranceId)) {
    classes.push("transcript-line--linked");
  }
  return classes.join(" ");
}

/** Tooltip explaining why a line is lit, or undefined when it is not linked. */
export function describeLink(
  index: ReturnType<typeof buildLinkIndex>,
  noteBlockId: string,
  utteranceId: number,
): string | undefined {
  const link = index.detail.get(detailKey(noteBlockId, utteranceId));
  return link ? `Linked by ${explainLink(link)}` : undefined;
}

/** `mic` is the user, `system` is everyone else — SPEC section 4. */
function speaker(source: AudioSource): string {
  return source === "mic" ? "You" : "Them";
}

/** Meeting start, in the reader's locale. */
function formatDate(epochMs: number): string {
  return new Date(epochMs).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Wall-clock length, or a dash while a meeting is still open. */
function formatDuration(startedAt: number, endedAt: number | null): string {
  if (endedAt === null) return "—";
  const seconds = Math.max(0, Math.round((endedAt - startedAt) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
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
  const [lifecycle, setLifecycle] = useState<MeetingStateT>({ state: "idle" });
  const active =
    lifecycle.state === "recording" || lifecycle.state === "processing"
      ? lifecycle.meeting_id
      : null;
  const isRecording = lifecycle.state === "recording";
  const isProcessing = lifecycle.state === "processing";
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
  // Wall-clock instant the recording began, so note anchors and transcript
  // timestamps share a zero. Without that the linker would compare note times
  // against transcript times measured from a different origin.
  const recordingStartedAt = useRef<number | null>(null);
  const [transcriptOpen, setTranscriptOpen] = useState(true);
  const [highlighted, setHighlighted] = useState<number | null>(null);
  const [links, setLinks] = useState<StoredLink[]>([]);
  const [utterances, setUtterances] = useState<Utterance[]>([]);
  // Which side the pointer is on. Only one is ever set — the highlight is
  // driven from whichever pane the mouse is actually in.
  const [hoveredNote, setHoveredNote] = useState<string | null>(null);
  const [hoveredUtterance, setHoveredUtterance] = useState<number | null>(null);

  /**
   * Scrolls the transcript to a cited line and flashes it.
   *
   * A citation that cannot be followed is worse than none — it looks like
   * evidence. Opening the transcript first means the chip works even when the
   * pane is collapsed.
   */
  const revealUtterance = useCallback((utteranceId: number) => {
    setTranscriptOpen(true);
    setHighlighted(utteranceId);
    // After the pane has had a frame to render.
    requestAnimationFrame(() => {
      document
        .querySelector(`[data-utterance-id="${utteranceId}"]`)
        ?.scrollIntoView({ block: "center", behavior: "smooth" });
    });
  }, []);

  const elapsedMs = useCallback(
    () =>
      recordingStartedAt.current === null ? 0 : Date.now() - recordingStartedAt.current,
    [],
  );

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
        const current = await meetingState();
        if (cancelled) return;
        setLifecycle(current);
        const currentId =
          current.state === "recording" || current.state === "processing"
            ? current.meeting_id
            : null;

        const stored = await meetingsList();
        if (cancelled) return;
        setMeetings(stored);

        // Prefer the newest meeting that actually has a transcript. An empty
        // or interrupted one is the least useful thing to reopen into, and it
        // hides the fact that anything was ever saved.
        const latest = stored.find((m) => m.utteranceCount > 0);
        if (!currentId && latest) {
          const utterances = await meetingTranscript(latest.id);
          if (cancelled) return;
          setViewing(latest.id);
          setLines(
            utterances.map((u) => ({
              key: `stored-${u.id}`,
              // Without this the restored-on-launch transcript has lines with
              // no identity: citation chips find nothing to scroll to and the
              // link highlight has nothing to match. The open-a-meeting path
              // below always set it, so the bug only ever showed on a cold start.
              utteranceId: u.id,
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
        // Rust emits the lifecycle change itself; just reload the list.
        refreshMeetings();
      }
    },
    [refreshMeetings],
  );

  // Rust is the source of truth for the lifecycle; mirror it rather than
  // tracking it independently.
  useEffect(() => {
    let cancelled = false;
    let unsubscribe: UnlistenFn | null = null;
    onMeetingState((next) => setLifecycle(next)).then((fn) => {
      if (cancelled) fn();
      else unsubscribe = fn;
    });
    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  }, []);

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
      recordingStartedAt.current = Date.now();
      await meetingStart();
    });

  const stop = () =>
    guard(async () => {
      await meetingStop();
    });

  const rename = (meetingId: string, currentTitle: string) =>
    guard(async () => {
      const title = window.prompt("Rename meeting", currentTitle);
      if (title === null || title.trim() === "") return;
      await meetingRename(meetingId, title.trim());
      await refreshMeetings();
    });

  const remove = (meetingId: string, title: string) =>
    guard(async () => {
      // Deleting a meeting destroys its transcript, notes and audio, and there
      // is no undo — so this asks first.
      if (
        !window.confirm(`Delete "${title}"? Its transcript, notes and audio go too.`)
      ) {
        return;
      }
      await meetingDelete(meetingId);
      if (viewing === meetingId) {
        setViewing(null);
        setLines([]);
      }
      await refreshMeetings();
    });

  /** Loads a stored transcript — the proof that it survived a restart. */
  const open = (meetingId: string) =>
    guard(async () => {
      const utterances: Utterance[] = await meetingTranscript(meetingId);
      setViewing(meetingId);
      setLines(
        utterances.map((u) => ({
          key: `stored-${u.id}`,
          utteranceId: u.id,
          source: u.source,
          text: u.text,
          startMs: u.startMs,
          partial: false,
        })),
      );
    });

  const shown = viewing ?? active;

  const reloadLinks = useCallback(() => {
    if (!shown) {
      setLinks([]);
      setUtterances([]);
      return;
    }
    void meetingLinks(shown)
      .then(setLinks)
      .catch(() => setLinks([]));
    void meetingTranscript(shown)
      .then(setUtterances)
      .catch(() => setUtterances([]));
  }, [shown]);

  useEffect(reloadLinks, [reloadLinks]);

  // Indexing runs in the background after a meeting ends, so the links do not
  // exist yet when the meeting first appears. This is the nudge to pick them up.
  useEffect(() => {
    const handle = onMeetingIndexed(reloadLinks);
    return () => {
      void handle.then((off) => off?.());
    };
  }, [reloadLinks]);

  const linkIndex = useMemo(() => buildLinkIndex(links), [links]);
  const litUtterances = highlightedUtterances(linkIndex, hoveredNote, hoveredUtterance);
  const litNotes = highlightedNotes(linkIndex, hoveredNote, hoveredUtterance);

  return (
    <section className="card">
      <div className="card-head">
        <h2>Record</h2>
        {isRecording && <span className="pill pill--err">recording</span>}
        {isProcessing && <span className="pill pill--pending">finalising</span>}
        {!isRecording && !isProcessing && (
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
        <button onClick={stop} disabled={!isRecording}>
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

      <PanelView meetingId={viewing ?? active} onCitationClick={revealUtterance} />

      <div className="meeting-body">
        <Notepad
          meetingId={shown}
          elapsedMs={elapsedMs}
          highlightedBlocks={litNotes}
          onHoverBlock={(blockId) => {
            setHoveredNote(blockId);
            if (blockId !== null) {
              setHoveredUtterance(null);
            }
          }}
        />

        <div
          className={transcriptOpen ? "transcript" : "transcript transcript--closed"}
        >
          <div className="transcript-head">
            <span className="notepad-label">Transcript</span>
            <button
              className="link-button"
              onClick={() => setTranscriptOpen((open) => !open)}
              aria-expanded={transcriptOpen}
            >
              {transcriptOpen ? "Hide" : "Show"}
            </button>
          </div>
          {transcriptOpen && (
            <div className="log" ref={transcriptRef} data-testid="transcript">
              {lines.map((line) => (
                <div
                  className={transcriptLineClass(
                    line.utteranceId,
                    highlighted,
                    litUtterances,
                  )}
                  key={line.key}
                  data-utterance-id={line.utteranceId}
                  title={
                    line.utteranceId !== undefined && hoveredNote
                      ? describeLink(linkIndex, hoveredNote, line.utteranceId)
                      : undefined
                  }
                  onMouseEnter={() => {
                    if (line.utteranceId !== undefined) {
                      setHoveredUtterance(line.utteranceId);
                      setHoveredNote(null);
                    }
                  }}
                  onMouseLeave={() => setHoveredUtterance(null)}
                >
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
          )}
        </div>
      </div>

      <LinkTuner
        meetingId={shown}
        links={links}
        utterances={utterances}
        onRelinked={reloadLinks}
      />

      <div className="meetings">
        <div className="meetings-head">
          <span>Stored meetings</span>
          <button onClick={refreshMeetings}>Refresh</button>
        </div>
        {meetings.length === 0 && <p className="empty-note">Nothing recorded yet.</p>}
        {meetings.map((meeting) => {
          const title = meeting.title ?? meeting.id;
          return (
            <div
              key={meeting.id}
              className={viewing === meeting.id ? "meeting meeting--open" : "meeting"}
            >
              <button className="meeting-open" onClick={() => open(meeting.id)}>
                <span className="meeting-title">{title}</span>
                <span className="meeting-meta">
                  {formatDate(meeting.startedAt)} &middot;{" "}
                  {formatDuration(meeting.startedAt, meeting.endedAt)} &middot;{" "}
                  {meeting.utteranceCount} utterance
                  {meeting.utteranceCount === 1 ? "" : "s"} &middot; {meeting.status}
                  {meeting.audioPath ? " · audio saved" : ""}
                </span>
              </button>
              <span className="meeting-actions">
                <button
                  className="link-button"
                  onClick={() => rename(meeting.id, title)}
                >
                  Rename
                </button>
                <button
                  className="link-button"
                  onClick={() => remove(meeting.id, title)}
                >
                  Delete
                </button>
              </span>
            </div>
          );
        })}
      </div>
    </section>
  );
}
