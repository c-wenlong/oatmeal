import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DetectionPopup,
  primaryLabel,
  candidateHeadline,
  candidateReason,
} from "./DetectionPopup";
import {
  detectionAnswerApp,
  detectionCandidates,
  detectionJoin,
  detectionPendingQuestion,
  detectionRespond,
  onAppQuestion,
  onCandidates,
} from "../lib/tauri";
import type { AppQuestion, Candidate } from "../types";

vi.mock("../lib/tauri", () => ({
  detectionCandidates: vi.fn(),
  detectionJoin: vi.fn(),
  detectionPendingQuestion: vi.fn(),
  detectionRespond: vi.fn(),
  detectionAnswerApp: vi.fn(),
  onCandidates: vi.fn(),
  onAppQuestion: vi.fn(),
}));

const mockList = vi.mocked(detectionCandidates);
const mockPending = vi.mocked(detectionPendingQuestion);
const mockRespond = vi.mocked(detectionRespond);
const mockAnswer = vi.mocked(detectionAnswerApp);
const mockOnCandidates = vi.mocked(onCandidates);
const mockOnQuestion = vi.mocked(onAppQuestion);

let emitQuestion: (q: AppQuestion) => void;

function candidate(over: Partial<Candidate> = {}): Candidate {
  return {
    id: "c1",
    source: "mic",
    title: null,
    bundleId: "us.zoom.xos",
    appName: "Zoom",
    calendarEventId: null,
    joinUrl: null,
    atMs: 0,
    ...over,
  };
}

beforeEach(() => {
  for (const m of [
    mockList,
    mockPending,
    mockRespond,
    mockAnswer,
    mockOnCandidates,
    mockOnQuestion,
  ]) {
    m.mockReset();
  }
  mockList.mockResolvedValue([]);
  mockPending.mockResolvedValue(null);
  mockRespond.mockResolvedValue(null);
  mockAnswer.mockResolvedValue(undefined);
  mockOnCandidates.mockImplementation(async () => () => {});
  mockOnQuestion.mockImplementation(async (handler) => {
    emitQuestion = handler;
    return () => {};
  });
});

describe("candidateHeadline", () => {
  it("prefers a calendar title", () => {
    expect(candidateHeadline(candidate({ title: "Standup" }))).toBe("Standup");
  });

  it("falls back to the app when there is no title", () => {
    expect(candidateHeadline(candidate())).toBe("Zoom call");
  });

  it("still says something when nothing is known", () => {
    expect(candidateHeadline(candidate({ appName: null, bundleId: null }))).toBe(
      "Meeting starting",
    );
  });

  it("ignores a whitespace-only title", () => {
    expect(candidateHeadline(candidate({ title: "   " }))).toBe("Zoom call");
  });
});

describe("candidateReason", () => {
  it("explains why each source fired", () => {
    expect(candidateReason(candidate({ source: "calendar", appName: null }))).toBe(
      "From your calendar",
    );
    expect(candidateReason(candidate({ source: "mic" }))).toBe(
      "Zoom started using your microphone",
    );
    expect(candidateReason(candidate({ source: "manual" }))).toBe(
      "You asked to record",
    );
  });
});

describe("DetectionPopup", () => {
  it("offers rather than records", async () => {
    // The invariant for the whole feature: showing the popup must not start
    // anything. Only a click does.
    mockList.mockResolvedValue([candidate()]);
    render(<DetectionPopup />);

    expect(await screen.findByText("Zoom call")).toBeInTheDocument();
    expect(mockRespond).not.toHaveBeenCalled();
  });

  it("starts recording when asked", async () => {
    // Routed through `detectionJoin` now, which opens a link when there is one
    // and starts the recording either way — one action for one intention.
    mockList.mockResolvedValue([candidate()]);
    render(<DetectionPopup />);

    fireEvent.click(await screen.findByRole("button", { name: /start recording/i }));
    await waitFor(() => expect(detectionJoin).toHaveBeenCalledWith("c1", null));
  });

  it("can dismiss just this one", async () => {
    mockList.mockResolvedValue([candidate()]);
    render(<DetectionPopup />);

    fireEvent.click(await screen.findByRole("button", { name: /not now/i }));
    await waitFor(() => expect(mockRespond).toHaveBeenCalledWith("c1", "ignore"));
  });

  it("can refuse the app permanently", async () => {
    mockList.mockResolvedValue([candidate()]);
    render(<DetectionPopup />);

    fireEvent.click(await screen.findByRole("button", { name: /never for Zoom/i }));
    await waitFor(() => expect(mockRespond).toHaveBeenCalledWith("c1", "ignore_app"));
  });

  it("does not offer to blocklist an app it cannot name", async () => {
    // Calendar-only candidates have no bundle id, so there is nothing a rule
    // could be written about.
    mockList.mockResolvedValue([
      candidate({
        source: "calendar",
        bundleId: null,
        appName: null,
        title: "Standup",
      }),
    ]);
    render(<DetectionPopup />);

    await screen.findByText("Standup");
    expect(screen.queryByRole("button", { name: /never for/i })).toBeNull();
  });

  it("asks about an unknown app before offering anything", async () => {
    render(<DetectionPopup />);
    await waitFor(() => expect(mockOnQuestion).toHaveBeenCalled());

    emitQuestion({ bundleId: "com.wisprflow.app", appName: "Wispr Flow" });

    expect(await screen.findByTestId("app-question")).toBeInTheDocument();
    expect(
      screen.getByText(/Record Wispr Flow calls\?/i),
    ).toBeInTheDocument();
    // The dictation case: it must not have queued an offer.
    expect(screen.queryByRole("button", { name: /start recording/i })).toBeNull();
  });

  it("remembers a never answer", async () => {
    render(<DetectionPopup />);
    await waitFor(() => expect(mockOnQuestion).toHaveBeenCalled());
    emitQuestion({ bundleId: "com.wisprflow.app", appName: "Wispr Flow" });

    fireEvent.click(await screen.findByRole("button", { name: /^never$/i }));
    await waitFor(() =>
      expect(mockAnswer).toHaveBeenCalledWith("com.wisprflow.app", "Wispr Flow", false),
    );
  });

  it("remembers an always answer", async () => {
    render(<DetectionPopup />);
    await waitFor(() => expect(mockOnQuestion).toHaveBeenCalled());
    emitQuestion({ bundleId: "com.example.calls", appName: "Calls" });

    fireEvent.click(await screen.findByRole("button", { name: /^always$/i }));
    await waitFor(() =>
      expect(mockAnswer).toHaveBeenCalledWith("com.example.calls", "Calls", true),
    );
  });

  it("says how many more are waiting", async () => {
    mockList.mockResolvedValue([candidate(), candidate({ id: "c2" })]);
    render(<DetectionPopup />);
    expect(await screen.findByText(/1 more waiting/)).toBeInTheDocument();
  });

  it("shows nothing when the queue is empty", async () => {
    render(<DetectionPopup />);
    await waitFor(() => expect(mockList).toHaveBeenCalled());
    expect(screen.queryByTestId("detection-popup")).toBeNull();
  });

  it("shows a question raised before the window existed", async () => {
    // The popup window is created by the same code that raises the question, so
    // the event has already been sent by the time this can subscribe. Without
    // reading the stored value the window opens blank.
    mockPending.mockResolvedValue({
      bundleId: "com.wisprflow.app",
      appName: "Wispr Flow",
    });
    render(<DetectionPopup />);

    expect(await screen.findByTestId("app-question")).toBeInTheDocument();
  });
});

describe("primaryLabel", () => {
  it("offers to join when the calendar carried a link", () => {
    // Joining the call and recording it are one intention. Splitting them
    // means doing the second one late, from another window.
    expect(primaryLabel(candidate({ joinUrl: "https://meet.google.com/abc" }))).toBe(
      "Join and record",
    );
  });

  it("only offers to record when there is nothing to join", () => {
    // A mic activation has no link, and a button promising to join would open
    // nothing.
    expect(primaryLabel(candidate({ joinUrl: null }))).toBe("Start recording");
  });
});

describe("the offer is movable", () => {
  it("makes the surface a drag region but never the buttons", async () => {
    // An undecorated window has no titlebar; without a drag region the offer
    // is nailed to wherever macOS first put it. A button that is also a drag
    // handle swallows its own click.
    mockList.mockResolvedValue([candidate()]);
    render(<DetectionPopup />);

    const pill = await screen.findByTestId("detection-popup");
    expect(pill).toHaveAttribute("data-tauri-drag-region");
    for (const button of screen.getAllByRole("button")) {
      expect(button).not.toHaveAttribute("data-tauri-drag-region");
    }
  });
});

describe("joining", () => {
  it("opens the link and records in one action", async () => {
    mockList.mockResolvedValue([
      candidate({ joinUrl: "https://meet.google.com/abc", source: "calendar" }),
    ]);
    render(<DetectionPopup />);

    fireEvent.click(await screen.findByRole("button", { name: /Join and record/ }));
    await waitFor(() =>
      expect(detectionJoin).toHaveBeenCalledWith("c1", "https://meet.google.com/abc"),
    );
  });

  it("still records when there is no link to open", async () => {
    mockList.mockResolvedValue([candidate({ joinUrl: null })]);
    render(<DetectionPopup />);

    fireEvent.click(await screen.findByRole("button", { name: /Start recording/ }));
    await waitFor(() => expect(detectionJoin).toHaveBeenCalledWith("c1", null));
  });
});

describe("the offer window is one shape", () => {
  it("makes both branches draggable, and neither button a handle", async () => {
    // The question branch shipped without a drag region and without the pill
    // layout, so it was unmovable and its text was cut to "Record whe…".
    mockPending.mockResolvedValue({ bundleId: "us.zoom.xos", appName: "Zoom" });
    render(<DetectionPopup />);

    const pill = await screen.findByTestId("app-question");
    expect(pill).toHaveAttribute("data-tauri-drag-region");
    for (const button of screen.getAllByRole("button")) {
      expect(button).not.toHaveAttribute("data-tauri-drag-region");
    }
  });

  it("keeps both branches to one short line each", async () => {
    // jsdom cannot measure overflow, so this guards the input to it: the pill
    // is 520px with two buttons, which leaves room for a title of roughly this
    // length. The real check is the browser one — see docs/ui-checks.md.
    mockPending.mockResolvedValue({
      bundleId: "com.microsoft.teams2",
      appName: "Microsoft Teams",
    });
    render(<DetectionPopup />);

    const title = await screen.findByText(/Record Microsoft Teams calls\?/);
    expect(title.textContent!.length).toBeLessThanOrEqual(40);
    expect(screen.getByText(/Asked once, then remembered/).textContent!.length)
      .toBeLessThanOrEqual(40);
  });
});
