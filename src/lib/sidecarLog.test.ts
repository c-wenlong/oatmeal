import { describe, expect, it } from "vitest";
import { elapsedLabel, formatSupervisorEvent } from "./sidecarLog";
import type { SupervisorEvent } from "../types";

describe("formatSupervisorEvent", () => {
  it("labels mic and system utterances distinctly", () => {
    const mic = formatSupervisorEvent({
      kind: "event",
      event: { ev: "final", source: "mic", text: "hello", t0: 0, t1: 1, conf: null },
    });
    const system = formatSupervisorEvent({
      kind: "event",
      event: { ev: "final", source: "system", text: "hi", t0: 0, t1: 1, conf: null },
    });

    // Attribution is the point of two capture streams; if these ever rendered
    // identically the UI would be hiding the one thing it must show.
    expect(mic.tag).toBe("mic");
    expect(system.tag).toBe("system");
    expect(mic.tone).not.toBe(system.tone);
  });

  it("renders confidence as a percentage when present", () => {
    const line = formatSupervisorEvent({
      kind: "event",
      event: { ev: "final", source: "mic", text: "hello", t0: 0, t1: 1, conf: 0.934 },
    });
    expect(line.text).toBe("hello  (93%)");
  });

  it("omits confidence entirely when the model gave none", () => {
    const line = formatSupervisorEvent({
      kind: "event",
      event: { ev: "final", source: "mic", text: "hello", t0: 0, t1: 1, conf: null },
    });
    expect(line.text).toBe("hello");
  });

  it("marks partials as in-flight and finals as settled", () => {
    const partial = formatSupervisorEvent({
      kind: "event",
      event: { ev: "partial", source: "mic", text: "hel", t0: 0, t1: 1 },
    });
    const final = formatSupervisorEvent({
      kind: "event",
      event: { ev: "final", source: "mic", text: "hello", t0: 0, t1: 1, conf: null },
    });
    expect(partial.partial).toBe(true);
    expect(final.partial).toBe(false);
  });

  it("reports the handshake with its protocol version", () => {
    const line = formatSupervisorEvent({
      kind: "event",
      event: { ev: "ready", version: "0.1.0", protocol: 1 },
    });
    expect(line.text).toContain("v0.1.0");
    expect(line.text).toContain("protocol 1");
  });

  it("distinguishes a restart from a permanent exit", () => {
    const restarting = formatSupervisorEvent({
      kind: "exited",
      code: 9,
      restarting_in_ms: 200,
    });
    const permanent = formatSupervisorEvent({
      kind: "exited",
      code: 0,
      restarting_in_ms: null,
    });
    expect(restarting.text).toContain("restarting in 200ms");
    expect(permanent.text).toContain("not restarting");
  });

  it("handles an exit with no code", () => {
    // A killed process reports no exit code; the line must still read sensibly.
    const line = formatSupervisorEvent({
      kind: "exited",
      code: null,
      restarting_in_ms: 100,
    });
    expect(line.text).not.toContain("null");
    expect(line.text).toContain("restarting");
  });

  it("surfaces garbled lines with both the error and the payload", () => {
    const line = formatSupervisorEvent({
      kind: "garbled",
      line: "{not json",
      error: "expected value",
    });
    expect(line.tone).toBe("error");
    expect(line.text).toContain("{not json");
    expect(line.text).toContain("expected value");
  });

  it("formats every event variant without throwing", () => {
    const events: SupervisorEvent[] = [
      { kind: "spawned", pid: 123, attempt: 1 },
      { kind: "exited", code: 0, restarting_in_ms: null },
      { kind: "gave_up", reason: "too many crashes" },
      { kind: "garbled", line: "x", error: "y" },
      { kind: "event", event: { ev: "ready", version: "0.1.0", protocol: 1 } },
      {
        kind: "event",
        event: { ev: "partial", source: "mic", text: "a", t0: 0, t1: 1 },
      },
      {
        kind: "event",
        event: { ev: "final", source: "system", text: "b", t0: 0, t1: 1, conf: 0.5 },
      },
      { kind: "event", event: { ev: "level", mic: 0.1, system: 0.2 } },
      { kind: "event", event: { ev: "stopped", audio_path: null, duration_ms: 7900 } },
      { kind: "event", event: { ev: "error", message: "boom", fatal: true } },
      { kind: "event", event: { ev: "pong" } },
      {
        kind: "event",
        event: {
          ev: "permissions",
          microphone: "granted",
          screen_recording: "denied",
          needs_relaunch: false,
        },
      },
      {
        kind: "event",
        event: {
          ev: "model",
          name: "small.en",
          state: "downloading",
          progress: 0.42,
          message: null,
        },
      },
    ];

    for (const event of events) {
      const line = formatSupervisorEvent(event);
      expect(
        line.text.length,
        `empty text for ${JSON.stringify(event)}`,
      ).toBeGreaterThan(0);
      expect(line.tag.length).toBeGreaterThan(0);
    }
  });

  it("flags a permissions event as an error only when capture is blocked", () => {
    const ok = formatSupervisorEvent({
      kind: "event",
      event: {
        ev: "permissions",
        microphone: "granted",
        screen_recording: "granted",
        needs_relaunch: false,
      },
    });
    const blocked = formatSupervisorEvent({
      kind: "event",
      event: {
        ev: "permissions",
        microphone: "granted",
        screen_recording: "denied",
        needs_relaunch: false,
      },
    });
    expect(ok.tone).toBe("meta");
    expect(blocked.tone).toBe("error");
  });

  it("calls out a stale grant even though both read granted", () => {
    const line = formatSupervisorEvent({
      kind: "event",
      event: {
        ev: "permissions",
        microphone: "granted",
        screen_recording: "granted",
        needs_relaunch: true,
      },
    });
    expect(line.tone).toBe("error");
    expect(line.text).toMatch(/relaunch/i);
  });

  it("shows model download progress as a percentage", () => {
    const line = formatSupervisorEvent({
      kind: "event",
      event: {
        ev: "model",
        name: "small.en",
        state: "downloading",
        progress: 0.42,
        message: null,
      },
    });
    expect(line.text).toContain("42%");
    expect(line.partial).toBe(true);
  });

  it("renders a stopped duration in seconds", () => {
    const line = formatSupervisorEvent({
      kind: "event",
      event: { ev: "stopped", audio_path: null, duration_ms: 7900 },
    });
    expect(line.text).toContain("7.9s");
  });
});

describe("elapsedLabel", () => {
  it("formats a fixed-width elapsed time", () => {
    expect(elapsedLabel(1000, 1000)).toBe("000.00");
    expect(elapsedLabel(1000, 3500)).toBe("002.50");
  });

  it("never goes negative if clocks jitter backwards", () => {
    expect(elapsedLabel(5000, 4000)).toBe("000.00");
  });
});
