import type { LinkMethod, StoredLink } from "../types";

/**
 * Links, indexed both ways.
 *
 * The highlight is bidirectional — hovering a note lights its transcript lines,
 * hovering a line lights the notes that came from it — so both directions are
 * built once when links load rather than scanning the list on every mouse move.
 */
export interface LinkIndex {
  /** Utterance ids linked from a note block. */
  byNote: Map<string, number[]>;
  /** Note block ids linked from an utterance. */
  byUtterance: Map<number, string[]>;
  /** Every link, keyed `blockId:utteranceId`, for method and score lookups. */
  detail: Map<string, StoredLink>;
}

export function detailKey(noteBlockId: string, utteranceId: number): string {
  return `${noteBlockId}:${utteranceId}`;
}

export function buildLinkIndex(links: StoredLink[]): LinkIndex {
  const byNote = new Map<string, number[]>();
  const byUtterance = new Map<number, string[]>();
  const detail = new Map<string, StoredLink>();

  for (const link of links) {
    const key = detailKey(link.noteBlockId, link.utteranceId);
    // One pair can be linked by more than one method — an LLM citation
    // confirming what the clock already found. Keep the strongest, so the
    // hover tooltip reports the best evidence rather than whichever arrived
    // last.
    const existing = detail.get(key);
    if (existing && existing.score >= link.score) {
      continue;
    }
    if (!existing) {
      appendUnique(byNote, link.noteBlockId, link.utteranceId);
      appendUnique(byUtterance, link.utteranceId, link.noteBlockId);
    }
    detail.set(key, link);
  }

  return { byNote, byUtterance, detail };
}

function appendUnique<K, V>(map: Map<K, V[]>, key: K, value: V): void {
  const existing = map.get(key);
  if (!existing) {
    map.set(key, [value]);
  } else if (!existing.includes(value)) {
    existing.push(value);
  }
}

/**
 * Which transcript lines to light up, given whatever is currently hovered.
 *
 * Returns a Set so the render path is a membership test rather than a scan.
 */
export function highlightedUtterances(
  index: LinkIndex,
  hoveredNote: string | null,
  hoveredUtterance: number | null,
): Set<number> {
  const out = new Set<number>();
  if (hoveredNote) {
    for (const id of index.byNote.get(hoveredNote) ?? []) {
      out.add(id);
    }
  }
  // Hovering a line highlights the line itself, so the pointer target and the
  // highlight agree even when that line has no links at all.
  if (hoveredUtterance !== null) {
    out.add(hoveredUtterance);
  }
  return out;
}

export function highlightedNotes(
  index: LinkIndex,
  hoveredNote: string | null,
  hoveredUtterance: number | null,
): Set<string> {
  const out = new Set<string>();
  if (hoveredUtterance !== null) {
    for (const id of index.byUtterance.get(hoveredUtterance) ?? []) {
      out.add(id);
    }
  }
  if (hoveredNote) {
    out.add(hoveredNote);
  }
  return out;
}

const METHOD_LABEL: Record<LinkMethod, string> = {
  temporal: "clock",
  semantic: "meaning",
  llm: "cited",
};

/** Human-readable "why is this linked", for the hover tooltip and tuner. */
export function explainLink(link: StoredLink): string {
  return `${METHOD_LABEL[link.method]} · ${link.score.toFixed(2)}`;
}

/**
 * Counts links by method.
 *
 * The single most useful number when tuning: if `semantic` is zero, the
 * embedder is not running and the weights being adjusted do nothing.
 */
export function methodBreakdown(links: StoredLink[]): Record<LinkMethod, number> {
  const counts: Record<LinkMethod, number> = { temporal: 0, semantic: 0, llm: 0 };
  for (const link of links) {
    counts[link.method] += 1;
  }
  return counts;
}
