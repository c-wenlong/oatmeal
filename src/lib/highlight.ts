import type { Preview } from "../types";

/** A run of preview text, marked or not. */
export interface Segment {
  text: string;
  marked: boolean;
}

/**
 * Splits a preview into runs for rendering.
 *
 * Rust returns offsets rather than markup, so the highlight is applied here
 * with React elements and never by injecting HTML — a transcript is arbitrary
 * user-adjacent text, and building a string with `<mark>` in it is how a
 * search result becomes an injection.
 *
 * Offsets are **character** indices. `Array.from` is used rather than `slice`
 * so an emoji or an accented character counts as one, matching what Rust
 * measured; `String.prototype.slice` counts UTF-16 code units and would drift
 * on any transcript containing one.
 */
export function segments(preview: Preview): Segment[] {
  const characters = Array.from(preview.text);
  const out: Segment[] = [];
  let cursor = 0;

  // Defensive: a malformed or overlapping span would otherwise produce
  // negative-length slices and silently drop text.
  const spans = [...preview.spans]
    .filter(([start, end]) => end > start && start >= 0 && end <= characters.length)
    .sort((a, b) => a[0] - b[0]);

  for (const [start, end] of spans) {
    if (start < cursor) continue;
    if (start > cursor) {
      out.push({ text: characters.slice(cursor, start).join(""), marked: false });
    }
    out.push({ text: characters.slice(start, end).join(""), marked: true });
    cursor = end;
  }

  if (cursor < characters.length) {
    out.push({ text: characters.slice(cursor).join(""), marked: false });
  }
  return out;
}

/** `mm:ss` from milliseconds, for pointing at a moment in a recording. */
export function timecode(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

/** Why a result matched, in words. */
export function matchLabel(kind: "keyword" | "semantic" | "both"): string {
  switch (kind) {
    case "keyword":
      return "words";
    case "semantic":
      return "meaning";
    case "both":
      return "words + meaning";
  }
}
