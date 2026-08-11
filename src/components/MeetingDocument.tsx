import { useCallback, useEffect, useRef, useState } from "react";
import { LiveTranscript } from "./LiveTranscript";
import {
  meetingDelete,
  meetingLinks,
  meetingRename,
  meetingTranscript,
  meetingsList,
  meetingState,
  onMeetingState,
  onSidecarEvent,
} from "../lib/tauri";
import type { MeetingState, MeetingSummary, StoredLink, Utterance } from "../types";
import { meetingTitle } from "./Library";
import { Notepad } from "./Notepad";
import { LinkTuner } from "./LinkTuner";
import { LinkedMoment } from "./LinkedMoment";
import { OverflowMenu } from "./OverflowMenu";
import { PanelView } from "./PanelView";

/**
 * A meeting, as a document.
 *
 * The harness rendered a meeting as a row of panels — a notes box beside a
 * monospace transcript, under a card of recording controls. Per
 * docs/ui-teardown.md this is the replacement: a title you could have written,
 * one row of quiet metadata, and then the page.
 *
 * The transcript is deliberately absent. It has not been deleted — the
 * workbench still shows it, and G35 brings it back here as a hover affordance
 * rather than a permanent pane. Shipping the pane now would mean building the
 * exact thing G35 exists to remove.
 */

/** Duration in words, or null while a meeting is still running. */
export function durationLabel(
  startedAt: number,
  endedAt: number | null,
): string | null {
  if (endedAt === null) return null;
  const minutes = Math.round((endedAt - startedAt) / 60_000);
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  // "2h" rather than "2h 0m": the zero carries no information.
  return rest === 0 ? `${hours}h` : `${hours}h ${rest}m`;
}

export function dateLabel(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

/** The metadata pills, in the order they should read. */
export function metaPills(meeting: MeetingSummary, live = 0): string[] {
  const pills = [dateLabel(meeting.startedAt)];
  const duration = durationLabel(meeting.startedAt, meeting.endedAt);
  if (duration) pills.push(duration);
  // `live` is what has arrived since this screen loaded. Without it the chip
  // reads "0 lines" through an entire recording that is visibly producing
  // them, which contradicts the transcript scrolling directly above it.
  const lines = meeting.utteranceCount + live;
  // Zero lines means nothing was transcribed, which is worth seeing rather
  // than hiding behind a plural.
  pills.push(lines === 1 ? "1 line" : `${lines} lines`);
  return pills;
}

/**
 * Where a note typed *now* should anchor, in ms since the meeting started.
 *
 * Every note block stores this, and the linker's temporal layer keys on it, so
 * it is not cosmetic. Editing a finished meeting a week later must not stamp
 * the note with a week — it anchors at the end of the meeting, the last moment
 * anything was actually said. A meeting still running gets live elapsed time.
 *
 * Passing 0 here, the obvious shortcut, would anchor every later edit to the
 * meeting's first second and silently pull its links to the opening remarks.
 */
export function noteAnchorMs(meeting: MeetingSummary, now: number): number {
  if (meeting.endedAt !== null) return meeting.endedAt - meeting.startedAt;
  return Math.max(0, now - meeting.startedAt);
}

export function MeetingDocument({
  meetingId,
  onBack,
}: {
  meetingId: string;
  onBack: () => void;
}) {
  /* Whether *this* meeting is the one being recorded. The lifecycle lives in
     Rust and can change from the tray or from detection, so it is subscribed
     to rather than assumed from however this screen was opened. */
  const [recording, setRecording] = useState(false);
  /* Lines that have arrived since this screen loaded. The summary is fetched
     once, so without this the chip stands still through a whole recording. */
  const [liveLines, setLiveLines] = useState(0);
  useEffect(() => {
    setLiveLines(0);
    let off: (() => void) | null = null;
    void onSidecarEvent((event) => {
      if (event.kind === "event" && event.event.ev === "final") {
        setLiveLines((was) => was + 1);
      }
    }).then((fn) => {
      off = fn;
    });
    return () => off?.();
  }, [meetingId]);
  useEffect(() => {
    let off: (() => void) | null = null;
    const apply = (state: MeetingState) =>
      setRecording(
        (state.state === "recording" || state.state === "processing") &&
          state.meeting_id === meetingId,
      );
    void meetingState()
      .then(apply)
      .catch(() => {});
    void onMeetingState(apply).then((fn) => {
      off = fn;
    });
    return () => off?.();
  }, [meetingId]);

  const [meeting, setMeeting] = useState<MeetingSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState(false);
  const [links, setLinks] = useState<StoredLink[]>([]);
  const [utterances, setUtterances] = useState<Utterance[]>([]);
  const [hovered, setHovered] = useState<string | null>(null);
  const [tuning, setTuning] = useState(false);
  const titleRef = useRef<HTMLInputElement | null>(null);

  const load = useCallback(async () => {
    try {
      const all = await meetingsList();
      const found = all.find((m) => m.id === meetingId);
      if (!found) {
        setError("That meeting is no longer in the database.");
        return;
      }
      setMeeting(found);
    } catch (err) {
      setError(String(err));
    }
  }, [meetingId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    // Both are needed before a hover can say anything, and both are read-only,
    // so a failure here costs the reveal and nothing else — the document still
    // opens and is still editable.
    void meetingLinks(meetingId)
      .then(setLinks)
      .catch(() => setLinks([]));
    void meetingTranscript(meetingId)
      .then(setUtterances)
      .catch(() => setUtterances([]));
  }, [meetingId]);

  async function commitTitle() {
    const next = titleRef.current?.value.trim();
    setEditingTitle(false);
    // An empty title is a rename to nothing, which would leave the row
    // unrecognisable in the library. Treat it as a cancel.
    if (!next || !meeting || next === meeting.title) return;
    await meetingRename(meetingId, next);
    await load();
  }

  if (error) return <p className="empty-note">{error}</p>;
  if (!meeting) return <p className="empty-note">Loading…</p>;

  return (
    <article className="document" data-testid="meeting-document">
      <div className="document-head">
        <button className="document-back" onClick={onBack}>
          ‹ Meetings
        </button>
        {/* The document's own menu. Link tuning belongs here rather than in
            global settings — it is scoped to this meeting's links — and so
            does deleting the meeting you are looking at. */}
        <OverflowMenu
          label="meeting actions"
          items={[
            {
              label: tuning ? "Hide link tuning" : "Tune linking",
              onSelect: () => setTuning((was) => !was),
            },
            {
              label: "Delete this meeting",
              onSelect: () => {
                void meetingDelete(meetingId).then(onBack);
              },
            },
          ]}
        />
      </div>

      {/* What the recorder is doing, while it does it — the first twelve
          seconds of a cold start are otherwise silent and indistinguishable
          from a broken app.

          Below the chrome, never above it: placed at the top of the article it
          pushed `‹ Meetings` and the title down the page as the text grew. */}
      <LiveTranscript recording={recording} />

      {editingTitle ? (
        <input
          ref={titleRef}
          className="document-title document-title--editing"
          defaultValue={meetingTitle(meeting)}
          aria-label="meeting title"
          autoFocus
          onBlur={() => void commitTitle()}
          onKeyDown={(e) => {
            if (e.key === "Enter") void commitTitle();
            if (e.key === "Escape") setEditingTitle(false);
          }}
        />
      ) : (
        <h1
          className="document-title"
          // A title you can fix in place, because the one detection guessed is
          // often nearly right and retyping it elsewhere is friction.
          onClick={() => setEditingTitle(true)}
        >
          {meetingTitle(meeting)}
        </h1>
      )}

      <div className="document-meta">
        {metaPills(meeting, liveLines).map((pill) => (
          <span key={pill} className="document-pill">
            {pill}
          </span>
        ))}
      </div>

      <PanelView meetingId={meetingId} onCitationClick={() => {}} />

      <div className="document-canvas">
        <Notepad
          meetingId={meetingId}
          elapsedMs={() => noteAnchorMs(meeting, Date.now())}
          variant="canvas"
          onHoverBlock={setHovered}
        />
      </div>

      {tuning && (
        <LinkTuner
          meetingId={meetingId}
          links={links}
          utterances={utterances}
          onRelinked={() => void meetingLinks(meetingId).then(setLinks)}
        />
      )}

      <LinkedMoment blockId={hovered} links={links} utterances={utterances} />
    </article>
  );
}
