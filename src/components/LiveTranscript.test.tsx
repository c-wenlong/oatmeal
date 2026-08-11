import { describe, expect, it } from "vitest";
import { isTranscript, liveStatus, type LiveState } from "./LiveTranscript";

const state = (over: Partial<LiveState> = {}): LiveState => ({
  model: "ready",
  partial: "",
  finals: 0,
  ...over,
});

describe("liveStatus", () => {
  it("explains the silence while the model loads", () => {
    // Measured on this machine: a cold start takes about twelve seconds, and
    // a 53-second recording produced its first line at the 25-second mark.
    // Showing only "recording" through that reads as a broken app.
    expect(liveStatus(state({ model: null }), true)).toMatch(
      /Loading the speech model/,
    );
    expect(liveStatus(state({ model: "loading" }), true)).toMatch(/few seconds/);
    expect(liveStatus(state({ model: "downloading" }), true)).toMatch(/Downloading/);
  });

  it("shows the words as they arrive", () => {
    expect(liveStatus(state({ partial: "Are you transcribing?" }), true)).toBe(
      "Are you transcribing?",
    );
  });

  it("says it is listening when there is nothing to show yet", () => {
    // Silence between sentences is not a failure, and a blank line is
    // indistinguishable from one.
    expect(liveStatus(state(), true)).toBe("Listening…");
    expect(liveStatus(state({ finals: 3 }), true)).toBe("Listening · 3 lines so far");
  });

  it("says nothing at all when not recording", () => {
    expect(liveStatus(state({ partial: "stale" }), false)).toBe("");
  });

  it("reports a failed model rather than pretending to listen", () => {
    expect(liveStatus(state({ model: "failed" }), true)).toMatch(/failed to load/);
  });

  it("prefers the loading note over stale text", () => {
    // A partial left over from a previous session must not imply the model is
    // ready for this one.
    expect(liveStatus(state({ model: "loading", partial: "old words" }), true)).toMatch(
      /Loading/,
    );
  });
});

describe("isTranscript", () => {
  it("distinguishes the model's words from the app's", () => {
    // They are styled differently: one is what was said, the other is Oatmeal
    // talking about itself.
    expect(isTranscript(state({ partial: "hello" }), true)).toBe(true);
    expect(isTranscript(state(), true)).toBe(false);
    expect(isTranscript(state({ model: "loading", partial: "hello" }), true)).toBe(
      false,
    );
    expect(isTranscript(state({ partial: "hello" }), false)).toBe(false);
  });
});
