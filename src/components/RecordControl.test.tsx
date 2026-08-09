import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  RecordControl,
  canStart,
  canStop,
  controlMode,
  litBars,
  modeLabel,
} from "./RecordControl";
import {
  meetingStart,
  meetingState,
  meetingStop,
  onSidecarEvent,
  sidecarSend,
} from "../lib/tauri";

vi.mock("../lib/tauri", () => ({
  meetingState: vi.fn(),
  meetingStart: vi.fn(),
  meetingStop: vi.fn(),
  sidecarSend: vi.fn(),
  onSidecarEvent: vi.fn(),
}));

const mockState = vi.mocked(meetingState);
const mockStart = vi.mocked(meetingStart);
const mockStop = vi.mocked(meetingStop);
const mockSend = vi.mocked(sidecarSend);
const mockEvents = vi.mocked(onSidecarEvent);

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  for (const m of [mockState, mockStart, mockStop, mockSend, mockEvents]) m.mockReset();
  mockState.mockResolvedValue({ state: "idle" });
  mockStart.mockResolvedValue("m1");
  mockStop.mockResolvedValue(undefined);
  mockSend.mockResolvedValue(undefined);
  mockEvents.mockResolvedValue(() => {});
});

afterEach(() => {
  vi.useRealTimers();
});

describe("canStop", () => {
  it("offers stop only while recording", () => {
    expect(canStop("recording")).toBe(true);
    expect(canStop("idle")).toBe(false);
    expect(canStop("armed")).toBe(false);
  });

  it("does not offer stop while processing", () => {
    // The sidecar was already asked to finish and is writing the file. A second
    // stop is either a no-op or a way to truncate the recording being saved.
    expect(canStop("processing")).toBe(false);
  });
});

describe("canStart", () => {
  it("allows starting from idle and armed", () => {
    expect(canStart("idle")).toBe(true);
    expect(canStart("armed")).toBe(true);
  });

  it("refuses to start on top of a recording", () => {
    expect(canStart("recording")).toBe(false);
  });

  it("refuses to start while the last meeting is still being written", () => {
    // Otherwise two recordings compete for the device and the user sees
    // nothing to suggest anything is wrong.
    expect(canStart("processing")).toBe(false);
  });
});

describe("modeLabel", () => {
  it("does not call armed 'recording'", () => {
    // Armed only fills the rolling buffer. Claiming the meeting is being kept
    // is the difference between a pre-roll and a lost conversation.
    expect(modeLabel("armed")).toBe("Ready");
    expect(modeLabel("recording")).toBe("Recording");
  });

  it("says processing is still doing something", () => {
    expect(modeLabel("processing")).toMatch(/Finishing/);
  });
});

describe("litBars", () => {
  it("lights nothing on silence", () => {
    expect(litBars(0)).toBe(0);
  });

  it("lights at least one bar on any real signal", () => {
    // Speech sits low in 0..1; rounding quiet speech to zero reads as "not
    // hearing you" through an entire meeting.
    expect(litBars(0.01)).toBeGreaterThanOrEqual(1);
  });

  it("clamps rather than overflowing the meter", () => {
    expect(litBars(5)).toBe(4);
  });

  it("survives a garbage level without rendering NaN bars", () => {
    expect(litBars(Number.NaN)).toBe(0);
    expect(litBars(-1)).toBe(0);
  });
});

describe("controlMode", () => {
  it("maps each lifecycle state", () => {
    expect(controlMode({ state: "idle" })).toBe("idle");
    expect(controlMode({ state: "recording", meeting_id: "m1" })).toBe("recording");
    expect(controlMode({ state: "processing", meeting_id: "m1" })).toBe("processing");
  });
});

describe("RecordControl", () => {
  it("arms before starting, so the pre-roll is not lost", async () => {
    render(<RecordControl />);
    fireEvent.click(await screen.findByLabelText("start recording"));

    await waitFor(() => expect(mockStart).toHaveBeenCalled());
    expect(mockSend).toHaveBeenCalledWith({ cmd: "arm" });
  });

  it("shows stop instead of start while recording", async () => {
    mockState.mockResolvedValue({ state: "recording", meeting_id: "m1" });
    render(<RecordControl />);

    expect(await screen.findByLabelText("stop recording")).toBeInTheDocument();
    expect(screen.queryByLabelText("start recording")).toBeNull();
  });

  it("stops when asked", async () => {
    mockState.mockResolvedValue({ state: "recording", meeting_id: "m1" });
    render(<RecordControl />);

    fireEvent.click(await screen.findByLabelText("stop recording"));
    await waitFor(() => expect(mockStop).toHaveBeenCalled());
  });

  it("offers neither start nor stop while processing", async () => {
    mockState.mockResolvedValue({ state: "processing", meeting_id: "m1" });
    render(<RecordControl />);

    expect(await screen.findByText(/Finishing/)).toBeInTheDocument();
    expect(screen.queryByLabelText("stop recording")).toBeNull();
    expect(screen.getByLabelText("start recording")).toBeDisabled();
  });

  it("follows a recording that something else started", async () => {
    // Detection can open a meeting with nobody touching this button. If the
    // control only trusted its own clicks it would sit there saying "Record"
    // through a live recording.
    mockState.mockResolvedValue({ state: "idle" });
    render(<RecordControl />);
    await screen.findByLabelText("start recording");

    mockState.mockResolvedValue({ state: "recording", meeting_id: "elsewhere" });
    await vi.advanceTimersByTimeAsync(1100);

    expect(await screen.findByLabelText("stop recording")).toBeInTheDocument();
  });

  it("reports a failure to start rather than looking like nothing happened", async () => {
    mockSend.mockRejectedValue("sidecar is not running");
    render(<RecordControl />);
    fireEvent.click(await screen.findByLabelText("start recording"));

    expect(await screen.findByText(/sidecar is not running/)).toBeInTheDocument();
  });
});
