import { describe, expect, it } from "vitest";
import {
  buildLinkIndex,
  detailKey,
  explainLink,
  highlightedNotes,
  highlightedUtterances,
  methodBreakdown,
} from "./links";
import type { StoredLink } from "../types";

function link(
  noteBlockId: string,
  utteranceId: number,
  method: StoredLink["method"] = "temporal",
  score = 0.5,
): StoredLink {
  return { noteBlockId, utteranceId, method, score };
}

describe("buildLinkIndex", () => {
  it("indexes in both directions", () => {
    const index = buildLinkIndex([link("b1", 10), link("b1", 11), link("b2", 11)]);

    expect(index.byNote.get("b1")).toEqual([10, 11]);
    expect(index.byUtterance.get(11)).toEqual(["b1", "b2"]);
  });

  it("keeps the strongest link when a pair is found by several methods", () => {
    // A citation confirming what the clock found is common; the tooltip should
    // report the better evidence, not whichever row came back last.
    const index = buildLinkIndex([
      link("b1", 10, "temporal", 0.4),
      link("b1", 10, "llm", 0.95),
    ]);

    expect(index.detail.get(detailKey("b1", 10))?.method).toBe("llm");
    expect(index.byNote.get("b1")).toEqual([10]);
  });

  it("does not let a weaker duplicate overwrite a stronger one", () => {
    const index = buildLinkIndex([
      link("b1", 10, "llm", 0.95),
      link("b1", 10, "temporal", 0.4),
    ]);

    expect(index.detail.get(detailKey("b1", 10))?.score).toBeCloseTo(0.95);
  });

  it("handles an empty list", () => {
    const index = buildLinkIndex([]);
    expect(index.byNote.size).toBe(0);
    expect(index.byUtterance.size).toBe(0);
  });
});

describe("highlighting", () => {
  const index = buildLinkIndex([link("b1", 10), link("b1", 11), link("b2", 20)]);

  it("lights a note's transcript lines when the note is hovered", () => {
    expect([...highlightedUtterances(index, "b1", null)].sort()).toEqual([10, 11]);
  });

  it("lights a line's notes when the line is hovered", () => {
    expect([...highlightedNotes(index, null, 10)]).toEqual(["b1"]);
  });

  it("highlights a hovered line even when nothing links to it", () => {
    // Otherwise pointing at an unlinked line looks like a broken feature
    // rather than a line nobody took notes on.
    expect(highlightedUtterances(index, null, 999).has(999)).toBe(true);
    expect(highlightedNotes(index, null, 999).size).toBe(0);
  });

  it("highlights a hovered note even when it links to nothing", () => {
    expect(highlightedNotes(index, "orphan", null).has("orphan")).toBe(true);
    expect(highlightedUtterances(index, "orphan", null).size).toBe(0);
  });

  it("highlights nothing when nothing is hovered", () => {
    expect(highlightedUtterances(index, null, null).size).toBe(0);
    expect(highlightedNotes(index, null, null).size).toBe(0);
  });
});

describe("explainLink", () => {
  it("names the method in plain words with the score", () => {
    expect(explainLink(link("b1", 1, "semantic", 0.732))).toBe("meaning · 0.73");
    expect(explainLink(link("b1", 1, "temporal", 0.4))).toBe("clock · 0.40");
    expect(explainLink(link("b1", 1, "llm", 1))).toBe("cited · 1.00");
  });
});

describe("methodBreakdown", () => {
  it("counts every method, including the ones with no links", () => {
    // A zero next to `semantic` is the signal that the embedder is not
    // running, which is the first thing to check when tuning does nothing.
    const counts = methodBreakdown([
      link("b1", 1, "temporal"),
      link("b2", 2, "temporal"),
      link("b3", 3, "llm"),
    ]);

    expect(counts).toEqual({ temporal: 2, semantic: 0, llm: 1 });
  });
});
