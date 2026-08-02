import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RecordCard } from "./RecordCard";
import {
  panelsList,
  templatesList,
  meetingDelete,
  meetingRename,
  meetingState,
  onMeetingState,
  meetingStart,
  meetingStop,
  meetingTranscript,
  meetingsList,
  notesLoad,
  notesSave,
  onSidecarEvent,
  sidecarSend,
  meetingLinks,
  onMeetingIndexed,
  linkParamsGet,
  linkParamsSet,
  meetingIndex,
} from "../lib/tauri";
import type {
  AudioSource,
  MeetingState as MeetingStateT,
  SupervisorEvent,
} from "../types";

vi.mock("../lib/tauri", () => ({
  onSidecarEvent: vi.fn(),
  sidecarSend: vi.fn(),
  meetingStart: vi.fn(),
  meetingStop: vi.fn(),
  meetingState: vi.fn(),
  onMeetingState: vi.fn(),
  meetingRename: vi.fn(),
  meetingDelete: vi.fn(),
  meetingsList: vi.fn(),
  meetingTranscript: vi.fn(),
  // The Notepad and PanelView children reach for these through the same
  // module boundary.
  notesLoad: vi.fn(),
  notesSave: vi.fn(),
  templatesList: vi.fn(),
  panelsList: vi.fn(),
  panelGenerate: vi.fn(),
  panelDelete: vi.fn(),
  // Linking (G18): the card loads links and subscribes to background indexing.
  meetingLinks: vi.fn(),
  onMeetingIndexed: vi.fn(),
  meetingIndex: vi.fn(),
  linkParamsGet: vi.fn(),
  linkParamsSet: vi.fn(),
}));

const mockOn = vi.mocked(onSidecarEvent);
const mockSend = vi.mocked(sidecarSend);
const mockStart = vi.mocked(meetingStart);
const mockStop = vi.mocked(meetingStop);
const mockState = vi.mocked(meetingState);
const mockOnState = vi.mocked(onMeetingState);
const mockRename = vi.mocked(meetingRename);
const mockDelete = vi.mocked(meetingDelete);
const mockList = vi.mocked(meetingsList);
const mockTranscript = vi.mocked(meetingTranscript);
const mockNotesLoad = vi.mocked(notesLoad);
const mockNotesSave = vi.mocked(notesSave);
const mockPanels = vi.mocked(panelsList);
const mockTemplateList = vi.mocked(templatesList);
const mockLinks = vi.mocked(meetingLinks);
const mockOnIndexed = vi.mocked(onMeetingIndexed);
const mockLinkParams = vi.mocked(linkParamsGet);

let subscriber: (event: SupervisorEvent) => void;
let meetingStateSubscriber: (state: MeetingStateT) => void;

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
    mockState,
    mockOnState,
    mockRename,
    mockDelete,
    mockList,
    mockTranscript,
    mockNotesLoad,
    mockNotesSave,
    mockPanels,
    mockTemplateList,
    mockLinks,
    mockOnIndexed,
    mockLinkParams,
    vi.mocked(meetingIndex),
    vi.mocked(linkParamsSet),
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
  mockState.mockResolvedValue({ state: "idle" });
  mockOnState.mockImplementation(async (handler) => {
    meetingStateSubscriber = handler;
    return () => {};
  });
  mockRename.mockResolvedValue(undefined);
  mockDelete.mockResolvedValue(undefined);
  mockList.mockResolvedValue([]);
  mockTranscript.mockResolvedValue([]);
  mockNotesLoad.mockResolvedValue([]);
  mockNotesSave.mockResolvedValue(undefined);
  mockPanels.mockResolvedValue([]);
  mockTemplateList.mockResolvedValue([]);
  mockLinks.mockResolvedValue([]);
  vi.mocked(linkParamsSet).mockResolvedValue(undefined);
  vi.mocked(meetingIndex).mockResolvedValue({ embedded: 0, links: 0, degraded: null });
  mockOnIndexed.mockImplementation(async () => () => {});
  mockLinkParams.mockResolvedValue({
    lookBackMs: 45_000,
    lookAheadMs: 10_000,
    alpha: 0.3,
    beta: 0.7,
    globalMargin: 0.15,
    minScore: 0.15,
    maxPerNote: 3,
  });
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
    expect(mockStart).toHaveBeenCalled();

    // The pill follows Rust's announcement rather than guessing optimistically —
    // otherwise a start that failed in the core would still look like it worked.
    act(() => meetingStateSubscriber({ state: "recording", meeting_id: "m123" }));
    expect(await screen.findByText("recording")).toBeInTheDocument();

    emit({
      kind: "event",
      event: { ev: "stopped", audio_path: "/tmp/a.m4a", duration_ms: 1000 },
    });
    act(() => meetingStateSubscriber({ state: "armed" }));

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
    mockState.mockResolvedValue({ state: "recording", meeting_id: "m999" });
    render(<RecordCard />);
    expect(await screen.findByText("recording")).toBeInTheDocument();
  });

  it("shows finalising, not idle, while a stop is still in flight", async () => {
    // The audio file is not closed and late utterances have not arrived yet.
    // Calling it idle would present a truncated recording as finished.
    mockState.mockResolvedValue({ state: "processing", meeting_id: "m999" });
    render(<RecordCard />);
    expect(await screen.findByText("finalising")).toBeInTheDocument();
    expect(screen.queryByText("idle")).not.toBeInTheDocument();
  });

  it("only offers Stop while actually recording", async () => {
    mockState.mockResolvedValue({ state: "processing", meeting_id: "m999" });
    render(<RecordCard />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^stop$/i })).toBeDisabled(),
    );
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

  it("shows each meeting's date and duration", async () => {
    mockList.mockResolvedValue([
      {
        id: "m1",
        title: "Quarterly planning",
        startedAt: 1_700_000_000_000,
        endedAt: 1_700_000_000_000 + 5 * 60 * 1000,
        status: "complete",
        audioPath: null,
        utteranceCount: 3,
      },
    ]);
    render(<RecordCard />);
    expect(await screen.findByText(/5m/)).toBeInTheDocument();
  });

  it("shows a dash for a meeting that never ended", async () => {
    mockList.mockResolvedValue([
      {
        id: "m1",
        title: "Open",
        startedAt: 1_700_000_000_000,
        endedAt: null,
        status: "interrupted",
        audioPath: null,
        utteranceCount: 0,
      },
    ]);
    render(<RecordCard />);
    expect(await screen.findByText(/—/)).toBeInTheDocument();
  });

  it("renames a meeting and reloads the list", async () => {
    mockList.mockResolvedValue([
      {
        id: "m1",
        title: "Untitled meeting",
        startedAt: 1,
        endedAt: 2,
        status: "complete",
        audioPath: null,
        utteranceCount: 1,
      },
    ]);
    vi.spyOn(window, "prompt").mockReturnValue("Quarterly planning");

    render(<RecordCard />);
    await userEvent.click(await screen.findByRole("button", { name: /rename/i }));

    expect(mockRename).toHaveBeenCalledWith("m1", "Quarterly planning");
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(2));
  });

  it("does not rename when the prompt is cancelled or blank", async () => {
    mockList.mockResolvedValue([
      {
        id: "m1",
        title: "Untitled meeting",
        startedAt: 1,
        endedAt: 2,
        status: "complete",
        audioPath: null,
        utteranceCount: 1,
      },
    ]);

    const prompt = vi.spyOn(window, "prompt").mockReturnValue(null);
    render(<RecordCard />);
    await userEvent.click(await screen.findByRole("button", { name: /rename/i }));
    expect(mockRename).not.toHaveBeenCalled();

    prompt.mockReturnValue("   ");
    await userEvent.click(screen.getByRole("button", { name: /rename/i }));
    expect(mockRename).not.toHaveBeenCalled();
  });

  it("asks before deleting, because there is no undo", async () => {
    mockList.mockResolvedValue([
      {
        id: "m1",
        title: "Quarterly planning",
        startedAt: 1,
        endedAt: 2,
        status: "complete",
        audioPath: "/tmp/a.m4a",
        utteranceCount: 1,
      },
    ]);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);

    render(<RecordCard />);
    await userEvent.click(await screen.findByRole("button", { name: /delete/i }));
    expect(confirm).toHaveBeenCalled();
    expect(mockDelete).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    await userEvent.click(screen.getByRole("button", { name: /delete/i }));
    expect(mockDelete).toHaveBeenCalledWith("m1");
  });

  it("surfaces a refusal to delete a live recording", async () => {
    mockList.mockResolvedValue([
      {
        id: "m1",
        title: "Live",
        startedAt: 1,
        endedAt: null,
        status: "recording",
        audioPath: null,
        utteranceCount: 1,
      },
    ]);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    mockDelete.mockRejectedValue(new Error("stop the recording before deleting it"));

    render(<RecordCard />);
    await userEvent.click(await screen.findByRole("button", { name: /delete/i }));

    expect(
      await screen.findByText(/stop the recording before deleting it/),
    ).toBeInTheDocument();
  });

  it("reflects a recording started outside this component", async () => {
    // Calendar detection (and the dev harness today) start meetings in Rust.
    // Showing "idle" through a live recording would be a plain lie, and Stop
    // would be unreachable.
    render(<RecordCard />);
    await waitFor(() => expect(mockOnState).toHaveBeenCalled());

    act(() => meetingStateSubscriber({ state: "recording", meeting_id: "elsewhere" }));

    expect(await screen.findByText("recording")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^stop$/i })).toBeEnabled();
  });

  it("follows the lifecycle through finalising back to idle", async () => {
    render(<RecordCard />);
    await waitFor(() => expect(mockOnState).toHaveBeenCalled());

    act(() => meetingStateSubscriber({ state: "recording", meeting_id: "m1" }));
    expect(await screen.findByText("recording")).toBeInTheDocument();

    act(() => meetingStateSubscriber({ state: "processing", meeting_id: "m1" }));
    expect(await screen.findByText("finalising")).toBeInTheDocument();

    act(() => meetingStateSubscriber({ state: "armed" }));
    expect(await screen.findByText("idle")).toBeInTheDocument();
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

describe("RecordCard linking (G18)", () => {
  /** A stored meeting with one note block linked to one of two lines. */
  function seedLinkedMeeting() {
    mockList.mockResolvedValue([
      {
        id: "m1",
        title: "Standup",
        startedAt: 1_700_000_000_000,
        endedAt: 1_700_000_060_000,
        utteranceCount: 2,
        status: "complete",
        audioPath: null,
      },
    ]);
    mockTranscript.mockResolvedValue([
      {
        id: 1,
        seq: 0,
        source: "system" as AudioSource,
        text: "the deadline for the migration is thursday",
        startMs: 18_000,
        endMs: 21_000,
        confidence: null,
      },
      {
        id: 2,
        seq: 1,
        source: "system" as AudioSource,
        text: "who is bringing lunch",
        startMs: 40_000,
        endMs: 43_000,
        confidence: null,
      },
    ]);
    mockNotesLoad.mockResolvedValue([
      {
        blockId: "b1",
        seq: 0,
        text: "deadline migration",
        firstTypedAtMs: 21_000,
        lastEditedAtMs: 21_000,
      },
    ]);
    mockLinks.mockResolvedValue([
      { noteBlockId: "b1", utteranceId: 1, method: "semantic", score: 0.82 },
    ]);
  }

  it("lights the linked note when a transcript line is hovered", async () => {
    seedLinkedMeeting();
    render(<RecordCard />);

    const line = await screen.findByText(/deadline for the migration/i);
    const row = line.closest("[data-utterance-id]") as HTMLElement;

    // Nothing lit until the pointer is actually on a line.
    expect(document.querySelector(".note-block--linked")).toBeNull();

    fireEvent.mouseEnter(row);

    await waitFor(() =>
      expect(document.querySelector(".note-block--linked")).not.toBeNull(),
    );
    expect(
      document.querySelector(".note-block--linked")?.getAttribute("data-block-id"),
    ).toBe("b1");
  });

  it("stops lighting the note once the pointer leaves", async () => {
    seedLinkedMeeting();
    render(<RecordCard />);

    const line = await screen.findByText(/deadline for the migration/i);
    const row = line.closest("[data-utterance-id]") as HTMLElement;

    fireEvent.mouseEnter(row);
    await waitFor(() =>
      expect(document.querySelector(".note-block--linked")).not.toBeNull(),
    );

    fireEvent.mouseLeave(row);
    await waitFor(() =>
      expect(document.querySelector(".note-block--linked")).toBeNull(),
    );
  });

  it("does not light a note from a line nothing links to", async () => {
    // The unlinked line still highlights itself, so the pointer target and the
    // highlight agree — but it must not drag an unrelated note along with it.
    seedLinkedMeeting();
    render(<RecordCard />);

    const line = await screen.findByText(/bringing lunch/i);
    const row = line.closest("[data-utterance-id]") as HTMLElement;

    fireEvent.mouseEnter(row);

    await waitFor(() => expect(row.className).toContain("transcript-line--linked"));
    expect(document.querySelector(".note-block--linked")).toBeNull();
  });

  it("shows how many links there are and by which method", async () => {
    seedLinkedMeeting();
    render(<RecordCard />);

    // Collapsed by default — the tuner is a debug surface, not the main event.
    const toggle = await screen.findByRole("button", { name: /tune linking/i });
    expect(toggle).toHaveTextContent("1 links");

    fireEvent.click(toggle);

    const breakdown = await screen.findByTestId("link-breakdown");
    expect(breakdown).toHaveTextContent("1 by meaning");
    expect(breakdown).toHaveTextContent("0 by clock");
  });

  it("does not blame a missing embedder when meaning simply agreed", async () => {
    // `method` records which layer decided a link, so zero-by-meaning means the
    // clock picked the same lines anyway. Claiming the model is missing there
    // would send the user chasing a problem they do not have.
    seedLinkedMeeting();
    mockLinks.mockResolvedValue([
      { noteBlockId: "b1", utteranceId: 1, method: "temporal", score: 0.4 },
    ]);
    render(<RecordCard />);

    fireEvent.click(await screen.findByRole("button", { name: /tune linking/i }));

    expect(await screen.findByText(/meaning changed no rankings/i)).toBeInTheDocument();
    expect(screen.queryByText(/no embedding model reachable/i)).toBeNull();
  });

  it("says so plainly when the embedder really was unreachable", async () => {
    seedLinkedMeeting();
    vi.mocked(meetingIndex).mockResolvedValue({
      embedded: 0,
      links: 1,
      degraded: "could not reach the embedding model at http://localhost:11434/v1",
    });
    render(<RecordCard />);

    fireEvent.click(await screen.findByRole("button", { name: /tune linking/i }));
    fireEvent.click(await screen.findByRole("button", { name: /apply and re-link/i }));

    expect(
      await screen.findByText(/no embedding model reachable/i),
    ).toBeInTheDocument();
  });

  it("re-links the meeting when the weights are applied", async () => {
    seedLinkedMeeting();
    vi.mocked(meetingIndex).mockResolvedValue({
      embedded: 3,
      links: 2,
      degraded: null,
    });
    render(<RecordCard />);

    fireEvent.click(await screen.findByRole("button", { name: /tune linking/i }));

    const slider = await screen.findByLabelText(/weight of time against meaning/i);
    fireEvent.change(slider, { target: { value: "0.5" } });
    fireEvent.click(screen.getByRole("button", { name: /apply and re-link/i }));

    await waitFor(() => expect(vi.mocked(linkParamsSet)).toHaveBeenCalled());
    // Alpha and beta are one budget: moving one must move the other, or every
    // score silently rescales and two sessions stop being comparable.
    expect(vi.mocked(linkParamsSet).mock.calls[0][0]).toMatchObject({
      alpha: 0.5,
      beta: 0.5,
    });
    expect(vi.mocked(meetingIndex)).toHaveBeenCalledWith("m1");
    // And the links are reloaded, so the list reflects what just happened.
    await waitFor(() => expect(mockLinks.mock.calls.length).toBeGreaterThan(1));
  });
});
