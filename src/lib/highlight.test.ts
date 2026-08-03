import { describe, expect, it } from "vitest";
import { matchLabel, segments, timecode } from "./highlight";
import type { Preview } from "../types";

function preview(text: string, spans: [number, number][]): Preview {
  return { text, spans, truncatedStart: false, truncatedEnd: false };
}

describe("segments", () => {
  it("splits a preview around its marked run", () => {
    const out = segments(preview("the deadline is Thursday", [[4, 12]]));
    expect(out).toEqual([
      { text: "the ", marked: false },
      { text: "deadline", marked: true },
      { text: " is Thursday", marked: false },
    ]);
  });

  it("reassembles into exactly the original text", () => {
    // The property that matters: highlighting must never lose or duplicate a
    // character of transcript.
    const original = "the deadline is Thursday and the migration is late";
    const out = segments(
      preview(original, [
        [4, 12],
        [30, 39],
      ]),
    );
    expect(out.map((s) => s.text).join("")).toBe(original);
  });

  it("handles a mark at the very start and end", () => {
    expect(segments(preview("abc", [[0, 3]]))).toEqual([{ text: "abc", marked: true }]);
  });

  it("returns one unmarked run when nothing matched", () => {
    expect(segments(preview("nothing here", []))).toEqual([
      { text: "nothing here", marked: false },
    ]);
  });

  it("counts characters, not UTF-16 units", () => {
    // Rust measured in characters. Using `slice` here would drift by one for
    // every emoji and highlight the wrong words after it.
    const text = "🎉 the deadline";
    const start = Array.from(text).indexOf("d");
    const out = segments(preview(text, [[start, start + 8]]));
    expect(out.find((s) => s.marked)?.text).toBe("deadline");
    expect(out.map((s) => s.text).join("")).toBe(text);
  });

  it("survives a span past the end of the text", () => {
    // Defensive: a malformed span must not drop text or throw.
    const out = segments(preview("short", [[2, 99]]));
    expect(out.map((s) => s.text).join("")).toBe("short");
  });

  it("survives an inverted span", () => {
    const out = segments(preview("short", [[4, 1]]));
    expect(out.map((s) => s.text).join("")).toBe("short");
  });

  it("ignores a span that overlaps one already applied", () => {
    const out = segments(
      preview("abcdef", [
        [0, 3],
        [1, 4],
      ]),
    );
    expect(out.map((s) => s.text).join("")).toBe("abcdef");
  });

  it("handles empty text", () => {
    expect(segments(preview("", []))).toEqual([]);
  });
});

describe("timecode", () => {
  it("formats a moment in a recording", () => {
    expect(timecode(0)).toBe("00:00");
    expect(timecode(65_000)).toBe("01:05");
    expect(timecode(3_600_000)).toBe("60:00");
  });

  it("does not render a negative clock", () => {
    expect(timecode(-1)).toBe("00:00");
  });
});

describe("matchLabel", () => {
  it("says why a result matched", () => {
    expect(matchLabel("keyword")).toBe("words");
    expect(matchLabel("semantic")).toBe("meaning");
    expect(matchLabel("both")).toBe("words + meaning");
  });
});
