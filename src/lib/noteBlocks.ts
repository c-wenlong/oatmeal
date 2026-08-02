import type { NoteBlock } from "../types";

/**
 * One top-level block as the editor currently sees it. `blockId` is assigned by
 * the editor extension and stays with the block for its whole life.
 */
export interface EditorBlock {
  blockId: string;
  text: string;
}

/**
 * Reconciles what the editor shows against what we already recorded.
 *
 * Kept pure and separate from the editor so the one rule that matters is
 * testable without mounting ProseMirror: **`firstTypedAtMs` is stamped once, on
 * the keystroke that first put text in a block, and never moves again.** It is
 * the anchor the temporal linker keys on (SPEC section 7) — if an edit could
 * shift it, a note would silently re-point at a different moment in the
 * transcript.
 *
 * @param blocks    current editor contents, in display order
 * @param previous  what we last recorded, keyed by blockId
 * @param elapsedMs milliseconds since the meeting started
 */
export function reconcileBlocks(
  blocks: EditorBlock[],
  previous: Map<string, NoteBlock>,
  elapsedMs: number,
): NoteBlock[] {
  return blocks.map((block, index) => {
    const existing = previous.get(block.blockId);
    const hasText = block.text.trim().length > 0;

    // An empty block has not been "typed in" yet — stamping it on creation
    // would anchor to the moment the user pressed Enter rather than the moment
    // they actually wrote something.
    const firstTypedAtMs = existing?.firstTypedAtMs ?? (hasText ? elapsedMs : null);

    // Only a genuine text change advances last-edited; re-saving unchanged
    // notes (autosave ticks, reordering) must not churn it.
    const textChanged = existing === undefined || existing.text !== block.text;
    const lastEditedAtMs = textChanged
      ? hasText || existing !== undefined
        ? elapsedMs
        : null
      : (existing?.lastEditedAtMs ?? null);

    return {
      blockId: block.blockId,
      seq: index,
      text: block.text,
      firstTypedAtMs,
      lastEditedAtMs,
    };
  });
}

/**
 * Blocks worth persisting.
 *
 * Trailing empties are an artifact of how editors work — there is almost always
 * a blank paragraph at the end — and saving them would give the linker empty
 * anchors to chew on. A blank line *between* two written blocks is deliberate
 * spacing, so it stays.
 */
export function persistableBlocks(blocks: NoteBlock[]): NoteBlock[] {
  let end = blocks.length;
  while (end > 0 && blocks[end - 1].text.trim().length === 0) {
    end -= 1;
  }
  return blocks.slice(0, end).map((block, index) => ({ ...block, seq: index }));
}

export function blocksById(blocks: NoteBlock[]): Map<string, NoteBlock> {
  return new Map(blocks.map((block) => [block.blockId, block]));
}

/** True when the notepad differs from what was last saved. */
export function hasUnsavedChanges(next: NoteBlock[], saved: NoteBlock[]): boolean {
  if (next.length !== saved.length) return true;
  return next.some((block, index) => {
    const other = saved[index];
    return (
      other === undefined ||
      block.blockId !== other.blockId ||
      block.text !== other.text ||
      block.seq !== other.seq
    );
  });
}
