import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  LinkedMoment,
  bestLink,
  methodLabel,
  offsetLabel,
  speakerLabel,
  utteranceById,
} from "./LinkedMoment";
import type { StoredLink, Utterance } from "../types";

function link(over: Partial<StoredLink> = {}): StoredLink {
  return {
    noteBlockId: "b1",
    utteranceId: 10,
    method: "semantic",
    score: 0.8,
    ...over,
  };
}

function utterance(over: Partial<Utterance> = {}): Utterance {
  return {
    id: 10,
    seq: 3,
    source: "system",
    text: "We should ship on Thursday.",
    startMs: 754_000,
    endMs: 758_000,
    confidence: 0.9,
    ...over,
  };
}

describe("bestLink", () => {
  it("ignores links belonging to other blocks", () => {
    const found = bestLink([link({ noteBlockId: "other" })], "b1");
    expect(found).toBeNull();
  });

  it("picks the strongest of several", () => {
    // A block carries the windowed best plus a global semantic catch. Showing
    // the weaker one would surface the guess rather than the answer.
    const found = bestLink(
      [
        link({ utteranceId: 1, score: 0.3 }),
        link({ utteranceId: 2, score: 0.9 }),
        link({ utteranceId: 3, score: 0.5 }),
      ],
      "b1",
    );
    expect(found?.utteranceId).toBe(2);
  });

  it("breaks ties the same way every time", () => {
    // Otherwise the same hover shows a different line depending on the order
    // rows came back from the database.
    const links = [
      link({ utteranceId: 9, score: 0.5 }),
      link({ utteranceId: 4, score: 0.5 }),
    ];
    expect(bestLink(links, "b1")?.utteranceId).toBe(4);
    expect(bestLink([...links].reverse(), "b1")?.utteranceId).toBe(4);
  });

  it("is null when the block has no links at all", () => {
    expect(bestLink([], "b1")).toBeNull();
  });
});

describe("offsetLabel", () => {
  it("reads as minutes and seconds", () => {
    expect(offsetLabel(754_000)).toBe("12:34");
  });

  it("pads seconds so the column does not jump", () => {
    expect(offsetLabel(65_000)).toBe("1:05");
  });

  it("grows an hours field rather than counting to 90 minutes", () => {
    expect(offsetLabel(3_725_000)).toBe("1:02:05");
  });

  it("does not render a negative time", () => {
    expect(offsetLabel(-5_000)).toBe("0:00");
  });
});

describe("speakerLabel", () => {
  it("uses the same words as the transcript", () => {
    expect(speakerLabel("mic")).toBe("You");
    expect(speakerLabel("system")).toBe("Them");
  });
});

describe("methodLabel", () => {
  it("says how the link was decided, in plain words", () => {
    // A link the clock produced and a link meaning produced deserve different
    // amounts of trust, and only the user can judge whether it landed.
    expect(methodLabel("temporal")).toBe("by timing");
    expect(methodLabel("semantic")).toBe("by meaning");
    expect(methodLabel("llm")).toMatch(/summary/);
  });

  it("covers every method the linker can produce", () => {
    // The union is temporal | semantic | llm. A method with no label would
    // render as nothing at all, and the reveal would silently lose its
    // provenance line.
    for (const method of ["temporal", "semantic", "llm"] as const) {
      expect(methodLabel(method)).not.toBe("");
    }
  });
});

describe("utteranceById", () => {
  it("returns null rather than undefined for a missing line", () => {
    expect(utteranceById([utterance()], 999)).toBeNull();
  });
});

describe("LinkedMoment", () => {
  it("shows nothing when nothing is hovered", () => {
    render(<LinkedMoment blockId={null} links={[link()]} utterances={[utterance()]} />);
    expect(screen.queryByTestId("linked-moment")).toBeNull();
  });

  it("reveals the moment a note came from", () => {
    render(<LinkedMoment blockId="b1" links={[link()]} utterances={[utterance()]} />);
    expect(screen.getByText("We should ship on Thursday.")).toBeInTheDocument();
    expect(screen.getByText("Them")).toBeInTheDocument();
    expect(screen.getByText("12:34")).toBeInTheDocument();
    expect(screen.getByText("by meaning")).toBeInTheDocument();
  });

  it("shows nothing for a block with no link, rather than an empty card", () => {
    render(
      <LinkedMoment blockId="unlinked" links={[link()]} utterances={[utterance()]} />,
    );
    expect(screen.queryByTestId("linked-moment")).toBeNull();
  });

  it("shows nothing when the linked line is missing", () => {
    // A stale link, or a meeting whose audio retention swept the transcript.
    // Half a card is worse than none.
    render(<LinkedMoment blockId="b1" links={[link()]} utterances={[]} />);
    expect(screen.queryByTestId("linked-moment")).toBeNull();
  });
});
