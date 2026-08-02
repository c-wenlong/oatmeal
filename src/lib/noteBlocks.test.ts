import { describe, expect, it } from "vitest";
import {
  blocksById,
  hasUnsavedChanges,
  persistableBlocks,
  reconcileBlocks,
  type EditorBlock,
} from "./noteBlocks";
import type { NoteBlock } from "../types";

function editor(...pairs: [string, string][]): EditorBlock[] {
  return pairs.map(([blockId, text]) => ({ blockId, text }));
}

describe("reconcileBlocks", () => {
  it("stamps the anchor on the keystroke that first adds text", () => {
    const result = reconcileBlocks(editor(["b1", "deadline"]), new Map(), 5_000);
    expect(result[0].firstTypedAtMs).toBe(5_000);
  });

  it("does not stamp an empty block", () => {
    // Pressing Enter creates a block; anchoring there would point the linker at
    // the moment a line was opened rather than the moment something was said.
    const result = reconcileBlocks(editor(["b1", "   "]), new Map(), 5_000);
    expect(result[0].firstTypedAtMs).toBeNull();
  });

  it("stamps a previously empty block once it gets text", () => {
    const first = reconcileBlocks(editor(["b1", ""]), new Map(), 1_000);
    const second = reconcileBlocks(
      editor(["b1", "now typed"]),
      blocksById(first),
      9_000,
    );
    expect(second[0].firstTypedAtMs).toBe(9_000);
  });

  it("never moves an anchor once set", () => {
    // The single most important rule in the notepad.
    const first = reconcileBlocks(editor(["b1", "deadline"]), new Map(), 5_000);
    const second = reconcileBlocks(
      editor(["b1", "deadline is the 14th"]),
      blocksById(first),
      90_000,
    );
    expect(second[0].firstTypedAtMs).toBe(5_000);
    expect(second[0].lastEditedAtMs).toBe(90_000);
  });

  it("keeps every anchor when a block is inserted in the middle", () => {
    const first = reconcileBlocks(
      editor(["b1", "first"], ["b2", "second"]),
      new Map(),
      1_000,
    );
    const second = reconcileBlocks(
      editor(["b1", "first"], ["b3", "inserted"], ["b2", "second"]),
      blocksById(first),
      50_000,
    );

    const byId = blocksById(second);
    expect(byId.get("b1")!.firstTypedAtMs).toBe(1_000);
    expect(byId.get("b2")!.firstTypedAtMs).toBe(1_000);
    expect(byId.get("b3")!.firstTypedAtMs).toBe(50_000);
  });

  it("renumbers seq to match display order", () => {
    const first = reconcileBlocks(
      editor(["b1", "first"], ["b2", "second"]),
      new Map(),
      1_000,
    );
    const swapped = reconcileBlocks(
      editor(["b2", "second"], ["b1", "first"]),
      blocksById(first),
      2_000,
    );
    expect(swapped.map((b) => [b.blockId, b.seq])).toEqual([
      ["b2", 0],
      ["b1", 1],
    ]);
  });

  it("does not churn last-edited when nothing changed", () => {
    // Autosave ticks and reorders must not make every block look freshly edited.
    const first = reconcileBlocks(editor(["b1", "text"]), new Map(), 1_000);
    const again = reconcileBlocks(editor(["b1", "text"]), blocksById(first), 80_000);
    expect(again[0].lastEditedAtMs).toBe(1_000);
  });

  it("handles an emptied block without losing its anchor", () => {
    // Clearing a line but keeping it is an edit, not a new block.
    const first = reconcileBlocks(editor(["b1", "typed"]), new Map(), 1_000);
    const cleared = reconcileBlocks(editor(["b1", ""]), blocksById(first), 7_000);
    expect(cleared[0].firstTypedAtMs).toBe(1_000);
    expect(cleared[0].text).toBe("");
  });

  it("returns nothing for an empty document", () => {
    expect(reconcileBlocks([], new Map(), 1_000)).toEqual([]);
  });
});

describe("persistableBlocks", () => {
  const make = (blockId: string, text: string, seq: number): NoteBlock => ({
    blockId,
    seq,
    text,
    firstTypedAtMs: null,
    lastEditedAtMs: null,
  });

  it("drops the trailing empty paragraph editors always leave", () => {
    const result = persistableBlocks([make("b1", "real note", 0), make("b2", "", 1)]);
    expect(result).toHaveLength(1);
    expect(result[0].blockId).toBe("b1");
  });

  it("drops several trailing empties", () => {
    const result = persistableBlocks([
      make("b1", "real", 0),
      make("b2", "  ", 1),
      make("b3", "", 2),
    ]);
    expect(result).toHaveLength(1);
  });

  it("keeps a deliberate blank line between written blocks", () => {
    // Spacing inside notes is intentional; only the trailing artifact goes.
    const result = persistableBlocks([
      make("b1", "before", 0),
      make("b2", "", 1),
      make("b3", "after", 2),
    ]);
    expect(result).toHaveLength(3);
  });

  it("renumbers after trimming so seq stays contiguous", () => {
    const result = persistableBlocks([
      make("b1", "a", 5),
      make("b2", "b", 9),
      make("b3", "", 12),
    ]);
    expect(result.map((b) => b.seq)).toEqual([0, 1]);
  });

  it("returns nothing for an entirely empty notepad", () => {
    expect(persistableBlocks([make("b1", "", 0), make("b2", "  ", 1)])).toEqual([]);
  });
});

describe("hasUnsavedChanges", () => {
  const make = (blockId: string, text: string, seq = 0): NoteBlock => ({
    blockId,
    seq,
    text,
    firstTypedAtMs: null,
    lastEditedAtMs: null,
  });

  it("is false when nothing moved", () => {
    const saved = [make("b1", "text")];
    expect(hasUnsavedChanges([make("b1", "text")], saved)).toBe(false);
  });

  it("notices edited text, new blocks, deletions and reorders", () => {
    const saved = [make("b1", "text", 0), make("b2", "other", 1)];
    expect(
      hasUnsavedChanges([make("b1", "changed", 0), make("b2", "other", 1)], saved),
    ).toBe(true);
    expect(hasUnsavedChanges([make("b1", "text", 0)], saved)).toBe(true);
    expect(
      hasUnsavedChanges([make("b2", "other", 0), make("b1", "text", 1)], saved),
    ).toBe(true);
  });

  it("treats an empty notepad against saved content as a change", () => {
    expect(hasUnsavedChanges([], [make("b1", "text")])).toBe(true);
  });
});
