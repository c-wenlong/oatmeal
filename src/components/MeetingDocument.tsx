import { useCallback, useEffect, useRef, useState } from "react";
import { meetingRename, meetingsList } from "../lib/tauri";
import type { MeetingSummary } from "../types";
import { meetingTitle } from "./Library";
import { Notepad } from "./Notepad";
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
export function metaPills(meeting: MeetingSummary): string[] {
  const pills = [dateLabel(meeting.startedAt)];
  const duration = durationLabel(meeting.startedAt, meeting.endedAt);
  if (duration) pills.push(duration);
  // Zero lines means nothing was transcribed, which is worth seeing rather
  // than hiding behind a plural.
  pills.push(
    meeting.utteranceCount === 1 ? "1 line" : `${meeting.utteranceCount} lines`,
  );
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
  const [meeting, setMeeting] = useState<MeetingSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState(false);
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
      <button className="document-back" onClick={onBack}>
        ‹ Meetings
      </button>

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
        {metaPills(meeting).map((pill) => (
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
        />
      </div>
    </article>
  );
}
