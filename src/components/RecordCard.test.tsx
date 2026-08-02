import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RecordCard } from "./RecordCard";
import {
  meetingActive,
  meetingStart,
  meetingStop,
  meetingTranscript,
  meetingsList,
  onSidecarEvent,
  sidecarSend,
} from "../lib/tauri";
import type { AudioSource, SupervisorEvent } from "../types";

vi.mock("../lib/tauri", () => ({
  onSidecarEvent: vi.fn(),
  sidecarSend: vi.fn(),
  meetingStart: vi.fn(),
  meetingStop: vi.fn(),
  meetingActive: vi.fn(),
  meetingsList: vi.fn(),
  meetingTranscript: vi.fn(),
}));

const mockOn = vi.mocked(onSidecarEvent);
const mockSend = vi.mocked(sidecarSend);
const mockStart = vi.mocked(meetingStart);
const mockStop = vi.mocked(meetingStop);
const mockActive = vi.mocked(meetingActive);
const mockList = vi.mocked(meetingsList);
const mockTranscript = vi.mocked(meetingTranscript);

let subscriber: (event: SupervisorEvent) => void;

function emit(event: SupervisorEvent) {
  act(() => subscriber(event));
}

function final(source: AudioSource, text: string, t0 = 0, t1 = 1000) {
  emit({
    kind: "event",
    event: { ev: "final", source, text, t0, t1, conf: null },
  });
}

// Block bodies: reset helpers return the mock, and vitest treats a function
// returned from a hook as a teardown callback.
beforeEach(() => {
  for (const m of [
    mockOn,
    mockSend,
    mockStart,
    mockStop,
    mockActive,
    mockList,
    mockTranscript,
  ]) {
    m.mockReset();
  }
  mockOn.mockImplementation(async (handler) => {
    subscriber = handler;
    return () => {};
  });
  mockSend.mockResolvedValue(undefined);
  mockStart.mockResolvedValue("m123");
  mockStop.mockResolvedValue(undefined);
  mockActive.mockResolvedValue(null);
  mockList.mockResolvedValue([]);
  mockTranscript.mockResolvedValue([]);
});

describe("RecordCard", () => {
  it("starts idle and does not record on mount", async () => {
    render(<RecordCard />);
    expect(screen.getByText("idle")).toBeInTheDocument();
    await waitFor(() => expect(mockList).toHaveBeenCalled());
    expect(mockStart).not.toHaveBeenCalled();
  });

  it("labels mic as You and system as Them", async () => {
    render(<RecordCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    final("system", "So the deadline is Thursday.");
    final("mic", "Got it.");

    const transcript = screen.getByTestId("transcript");
    // Attribution is the payoff of two capture streams — if this ever collapsed
    // to one label the feature would be silently gone.
    expect(await within(transcript).findByText("Them")).toBeInTheDocument();
    expect(within(transcript).getByText("You")).toBeInTheDocument();
    expect(within(transcript).getByText(/deadline is Thursday/)).toBeInTheDocument();
  });

  it("replaces a partial with its final rather than showing both", async () => {
    render(<RecordCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    emit({
      kind: "event",
      event: { ev: "partial", source: "mic", text: "so the dead", t0: 0, t1: 500 },
    });
    expect(await screen.findByText("so the dead")).toBeInTheDocument();

    final("mic", "So the deadline is Thursday.");

    await waitFor(() =>
      expect(screen.queryByText("so the dead")).not.toBeInTheDocument(),
    );
    expect(screen.getByText("So the deadline is Thursday.")).toBeInTheDocument();
  });

  it("keeps partials from the two sources independent", async () => {
    render(<RecordCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    emit({
      kind: "event",
      event: { ev: "partial", source: "mic", text: "mic text", t0: 0, t1: 1 },
    });
    emit({
      kind: "event",
      event: { ev: "partial", source: "system", text: "system text", t0: 0, t1: 1 },
    });

    // One stream's in-flight text must not evict the other's.
    expect(await screen.findByText("mic text")).toBeInTheDocument();
    expect(screen.getByText("system text")).toBeInTheDocument();
  });

  it("goes to recording on start and back to idle when the sidecar stops", async () => {
    render(<RecordCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    await userEvent.click(screen.getByRole("button", { name: /start recording/i }));
    expect(await screen.findByText("recording")).toBeInTheDocument();

    emit({
      kind: "event",
      event: { ev: "stopped", audio_path: "/tmp/a.m4a", duration_ms: 1000 },
    });

    expect(await screen.findByText("idle")).toBeInTheDocument();
  });

  it("reloads the meeting list once a recording stops", async () => {
    render(<RecordCard />);
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(1));

    emit({
      kind: "event",
      event: { ev: "stopped", audio_path: null, duration_ms: 0 },
    });

    // The just-finished meeting has to appear without a manual refresh.
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(2));
  });

  it("recovers an in-progress recording after a reload", async () => {
    // The sidecar keeps capturing across a window reload; showing "idle" would
    // be a lie and the stop button would be unreachable.
    mockActive.mockResolvedValue("m999");
    render(<RecordCard />);
    expect(await screen.findByText("recording")).toBeInTheDocument();
  });

  it("loads a stored transcript with attribution intact", async () => {
    mockList.mockResolvedValue([
      {
        id: "m1",
        title: "Quarterly planning",
        startedAt: 1,
        endedAt: 2,
        status: "complete",
        audioPath: "/tmp/a.m4a",
        utteranceCount: 2,
      },
    ]);
    mockTranscript.mockResolvedValue([
      {
        id: 1,
        seq: 0,
        source: "system",
        text: "Deadline is Thursday.",
        startMs: 0,
        endMs: 1000,
        confidence: 0.9,
      },
      {
        id: 2,
        seq: 1,
        source: "mic",
        text: "Got it.",
        startMs: 1000,
        endMs: 2000,
        confidence: 0.8,
      },
    ]);

    render(<RecordCard />);
    await userEvent.click(
      await screen.findByRole("button", { name: /Quarterly planning/ }),
    );

    const transcript = screen.getByTestId("transcript");
    expect(
      await within(transcript).findByText(/Deadline is Thursday/),
    ).toBeInTheDocument();
    // Reading back from SQLite must preserve who said what.
    expect(within(transcript).getByText("Them")).toBeInTheDocument();
    expect(within(transcript).getByText("You")).toBeInTheDocument();
  });

  it("reopens the newest meeting that has a transcript, not an empty one", async () => {
    // A crash leaves an interrupted, empty meeting at the top of the list.
    // Reopening into it would suggest nothing was ever saved.
    mockList.mockResolvedValue([
      {
        id: "ghost",
        title: "Untitled meeting",
        startedAt: 20,
        endedAt: 21,
        status: "interrupted",
        audioPath: null,
        utteranceCount: 0,
      },
      {
        id: "real",
        title: "Harness recording",
        startedAt: 10,
        endedAt: 11,
        status: "complete",
        audioPath: "/tmp/a.m4a",
        utteranceCount: 1,
      },
    ]);
    mockTranscript.mockResolvedValue([
      {
        id: 1,
        seq: 0,
        source: "system",
        text: "Deadline is Thursday.",
        startMs: 0,
        endMs: 1000,
        confidence: null,
      },
    ]);

    render(<RecordCard />);

    expect(await screen.findByText(/Deadline is Thursday/)).toBeInTheDocument();
    expect(mockTranscript).toHaveBeenCalledWith("real");
  });

  it("does not reopen anything when every meeting is empty", async () => {
    mockList.mockResolvedValue([
      {
        id: "ghost",
        title: "Untitled meeting",
        startedAt: 20,
        endedAt: 21,
        status: "interrupted",
        audioPath: null,
        utteranceCount: 0,
      },
    ]);
    render(<RecordCard />);
    await waitFor(() => expect(mockList).toHaveBeenCalled());
    expect(mockTranscript).not.toHaveBeenCalled();
  });

  it("arms without starting a recording", async () => {
    render(<RecordCard />);
    await userEvent.click(screen.getByRole("button", { name: /^arm$/i }));
    expect(mockSend).toHaveBeenCalledWith({ cmd: "arm" });
    expect(mockStart).not.toHaveBeenCalled();
  });

  it("surfaces a failure to start", async () => {
    mockStart.mockRejectedValue(new Error("sidecar is not running"));
    render(<RecordCard />);

    await userEvent.click(screen.getByRole("button", { name: /start recording/i }));
    expect(await screen.findByText(/sidecar is not running/)).toBeInTheDocument();
    expect(screen.getByText("idle")).toBeInTheDocument();
  });

  it("reports model download progress rather than looking hung", async () => {
    render(<RecordCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    emit({
      kind: "event",
      event: {
        ev: "model",
        name: "small.en",
        state: "downloading",
        progress: null,
        message: null,
      },
    });

    expect(await screen.findByText(/first run downloads it/i)).toBeInTheDocument();
  });
});
