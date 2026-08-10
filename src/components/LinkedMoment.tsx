import type { LinkMethod, StoredLink, Utterance } from "../types";

/**
 * The moment a note came from, revealed on hover.
 *
 * This is the product's reason to exist — G16 through G18, and the 90% the
 * benchmark measured — finally visible in the document. It replaces the
 * permanent transcript pane the harness kept beside the notes.
 *
 * **Why not a pane.** A pane that is always on screen is furniture, and
 * furniture is ignored; it also halves the width of the writing surface to show
 * something you look at rarely. Appearing where the eye already is — on the
 * line you are reading — makes the link noticeable precisely when it is
 * relevant. Reasoning recorded in docs/ui-teardown.md §5.
 */

/**
 * The link to show for a note block: the strongest one.
 *
 * A block can carry several — the windowed best plus a global semantic catch
 * (SPEC §7). Showing all of them turns a hover into a reading task, and showing
 * an arbitrary one would sometimes surface the weaker guess.
 */
export function bestLink(links: StoredLink[], blockId: string): StoredLink | null {
  let best: StoredLink | null = null;
  for (const link of links) {
    if (link.noteBlockId !== blockId) continue;
    // Ties break toward the earlier utterance, so the same hover always shows
    // the same line rather than depending on row order from the database.
    if (
      best === null ||
      link.score > best.score ||
      (link.score === best.score && link.utteranceId < best.utteranceId)
    ) {
      best = link;
    }
  }
  return best;
}

export function utteranceById(utterances: Utterance[], id: number): Utterance | null {
  return utterances.find((u) => u.id === id) ?? null;
}

/** `mm:ss` from the meeting's start, or `h:mm:ss` past an hour. */
export function offsetLabel(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const seconds = total % 60;
  const minutes = Math.floor(total / 60) % 60;
  const hours = Math.floor(total / 3600);
  const pad = (n: number) => `${n}`.padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${minutes}:${pad(seconds)}`;
}

/** Who was speaking, in the app's own words. */
export function speakerLabel(source: Utterance["source"]): string {
  // `mic` is this machine's microphone, so it is the user; everything the
  // machine played is the other side. Named the same way the transcript and
  // the spec do, so one vocabulary runs through the product.
  return source === "mic" ? "You" : "Them";
}

/**
 * How the link was decided, said plainly.
 *
 * Worth showing: a link the clock alone produced and a link meaning produced
 * deserve different amounts of trust, and the user is the one who can tell
 * whether it landed.
 */
export function methodLabel(method: LinkMethod): string {
  switch (method) {
    case "temporal":
      return "by timing";
    case "semantic":
      return "by meaning";
    case "llm":
      // The summariser named this line as its source (G14 citations), which is
      // a different kind of claim from the clock or the embedder agreeing.
      return "cited by the summary";
  }
}

export function LinkedMoment({
  blockId,
  links,
  utterances,
}: {
  blockId: string | null;
  links: StoredLink[];
  utterances: Utterance[];
}) {
  if (!blockId) return null;

  const link = bestLink(links, blockId);
  if (!link) return null;

  const utterance = utteranceById(utterances, link.utteranceId);
  // A link whose utterance is missing is a stale row, not something to render
  // half of. The transcript may still be loading, or retention may have swept
  // the meeting.
  if (!utterance) return null;

  return (
    <aside className="moment" data-testid="linked-moment">
      <div className="moment-head">
        <span className="moment-speaker">{speakerLabel(utterance.source)}</span>
        <span className="moment-time">{offsetLabel(utterance.startMs)}</span>
        <span className="moment-method">{methodLabel(link.method)}</span>
      </div>
      <p className="moment-text">{utterance.text}</p>
    </aside>
  );
}
