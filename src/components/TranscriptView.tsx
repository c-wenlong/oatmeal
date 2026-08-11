import type { Utterance } from "../types";

/**
 * The transcript, as a readable column.
 *
 * The document has always fetched these — the linker needs them — and never
 * shown them. So the one thing a user could check a summary against was the
 * one thing the app would not display, and a summary that dropped most of a
 * meeting looked the same as one that did not.
 */

/** `mm:ss` from the start of the meeting. */
export function offsetLabel(startMs: number): string {
  const total = Math.max(0, Math.round(startMs / 1000));
  return `${Math.floor(total / 60)}:${`${total % 60}`.padStart(2, "0")}`;
}

/** Who was speaking, in the two terms this app has. */
export function speakerLabel(source: string): string {
  // `mic` is the person whose notes these are; `system` is everyone else on
  // the call. "Mic" and "System" describe the plumbing, not the conversation.
  return source === "mic" ? "You" : "Them";
}

export function TranscriptView({
  utterances,
  onSeek,
}: {
  utterances: Utterance[];
  /** Reveals the moment elsewhere. Optional — the list reads on its own. */
  onSeek?: (utteranceId: number) => void;
}) {
  if (utterances.length === 0) {
    return (
      <p className="empty-note" data-testid="transcript-empty">
        Nothing was transcribed. If this meeting was recorded, the speech model may
        still have been loading when it started.
      </p>
    );
  }

  return (
    <div className="transcript" data-testid="transcript">
      {utterances.map((utterance) => (
        <div
          className={`transcript-line transcript-line--${utterance.source}`}
          key={utterance.id}
        >
          <button
            className="transcript-time"
            onClick={() => onSeek?.(utterance.id)}
            title={`Line ${utterance.id}`}
          >
            {offsetLabel(utterance.startMs)}
          </button>
          <span className="transcript-who">{speakerLabel(utterance.source)}</span>
          <span className="transcript-text">{utterance.text}</span>
        </div>
      ))}
    </div>
  );
}
