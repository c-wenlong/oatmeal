import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  MeetingDocument,
  dateLabel,
  durationLabel,
  metaPills,
  noteAnchorMs,
} from "./MeetingDocument";
import { meetingRename, meetingsList } from "../lib/tauri";
import type { MeetingSummary } from "../types";

vi.mock("../lib/tauri", () => ({
  meetingsList: vi.fn(),
  meetingRename: vi.fn(),
}));
// The document composes the real editor and panel view; both reach for Tauri
// and a ProseMirror DOM, neither of which this test is about.
vi.mock("./Notepad", () => ({ Notepad: () => <div data-testid="notepad" /> }));
vi.mock("./PanelView", () => ({ PanelView: () => <div data-testid="panels" /> }));

const mockList = vi.mocked(meetingsList);
const mockRename = vi.mocked(meetingRename);

const STARTED = new Date(2026, 7, 7, 14, 0).getTime();

function meeting(over: Partial<MeetingSummary> = {}): MeetingSummary {
  return {
    id: "m1",
    title: "Vendor call",
    startedAt: STARTED,
    endedAt: STARTED + 45 * 60_000,
    status: "complete",
    audioPath: null,
    utteranceCount: 120,
    ...over,
  };
}

beforeEach(() => {
  mockList.mockReset();
  mockRename.mockReset();
  mockList.mockResolvedValue([meeting()]);
  mockRename.mockResolvedValue(undefined);
});

describe("durationLabel", () => {
  it("is null while the meeting is still running", () => {
    // A running meeting has no duration yet; showing "0 min" would be a lie
    // that gets staler every second.
    expect(durationLabel(STARTED, null)).toBeNull();
  });

  it("reads in minutes under an hour", () => {
    expect(durationLabel(STARTED, STARTED + 45 * 60_000)).toBe("45 min");
  });

  it("reads in hours and minutes above one", () => {
    expect(durationLabel(STARTED, STARTED + 95 * 60_000)).toBe("1h 35m");
  });

  it("drops a zero minute remainder", () => {
    // "2h 0m" carries no information the "2h" does not.
    expect(durationLabel(STARTED, STARTED + 120 * 60_000)).toBe("2h");
  });

  it("does not round a short meeting down to nothing", () => {
    expect(durationLabel(STARTED, STARTED + 20_000)).toBe("under a minute");
  });
});

describe("metaPills", () => {
  it("reads date, duration, then size", () => {
    const pills = metaPills(meeting());
    expect(pills[0]).toMatch(/August 7/);
    expect(pills[1]).toBe("45 min");
    expect(pills[2]).toBe("120 lines");
  });

  it("omits duration entirely while recording rather than showing a blank", () => {
    const pills = metaPills(meeting({ endedAt: null }));
    expect(pills).toHaveLength(2);
    expect(pills.some((p) => /min|h /.test(p))).toBe(false);
  });

  it("says 1 line, not 1 lines", () => {
    expect(metaPills(meeting({ utteranceCount: 1 }))).toContain("1 line");
  });

  it("shows an empty transcript rather than hiding it", () => {
    // Zero lines means capture produced nothing, which the user needs to know.
    expect(metaPills(meeting({ utteranceCount: 0 }))).toContain("0 lines");
  });
});

describe("noteAnchorMs", () => {
  it("anchors a finished meeting at its end, however long ago that was", () => {
    // Editing a meeting a week later must not stamp the note with a week; the
    // linker keys on this and would drag the link far past anything said.
    const m = meeting({ endedAt: STARTED + 45 * 60_000 });
    const aWeekLater = STARTED + 7 * 86_400_000;
    expect(noteAnchorMs(m, aWeekLater)).toBe(45 * 60_000);
  });

  it("tracks live elapsed time while the meeting is running", () => {
    const m = meeting({ endedAt: null });
    expect(noteAnchorMs(m, STARTED + 10 * 60_000)).toBe(10 * 60_000);
  });

  it("never goes negative if the clock disagrees with the start", () => {
    const m = meeting({ endedAt: null });
    expect(noteAnchorMs(m, STARTED - 5000)).toBe(0);
  });

  it("is not simply zero", () => {
    // The shortcut this guards against: anchoring every later edit to the
    // meeting's first second, pulling its links to the opening remarks.
    const m = meeting({ endedAt: STARTED + 30 * 60_000 });
    expect(noteAnchorMs(m, STARTED)).toBeGreaterThan(0);
  });
});

describe("dateLabel", () => {
  it("spells the day out, since a document is read not scanned", () => {
    expect(dateLabel(STARTED)).toMatch(/Friday/);
    expect(dateLabel(STARTED)).toMatch(/August/);
  });
});

describe("MeetingDocument", () => {
  it("shows the title as the document heading", async () => {
    render(<MeetingDocument meetingId="m1" onBack={() => {}} />);
    expect(
      await screen.findByRole("heading", { name: "Vendor call" }),
    ).toBeInTheDocument();
  });

  it("renders the notes canvas and the panels", async () => {
    render(<MeetingDocument meetingId="m1" onBack={() => {}} />);
    expect(await screen.findByTestId("notepad")).toBeInTheDocument();
    expect(screen.getByTestId("panels")).toBeInTheDocument();
  });

  it("shows no transcript pane", async () => {
    // G35 brings the transcript back as a hover affordance. Shipping the pane
    // now would mean building the exact thing that goal exists to remove.
    render(<MeetingDocument meetingId="m1" onBack={() => {}} />);
    await screen.findByTestId("meeting-document");
    expect(screen.queryByText(/transcript/i)).toBeNull();
  });

  it("goes back to the library", async () => {
    const onBack = vi.fn();
    render(<MeetingDocument meetingId="m1" onBack={onBack} />);
    fireEvent.click(await screen.findByRole("button", { name: /Meetings/ }));
    expect(onBack).toHaveBeenCalled();
  });

  it("renames from the title itself", async () => {
    render(<MeetingDocument meetingId="m1" onBack={() => {}} />);
    fireEvent.click(await screen.findByRole("heading", { name: "Vendor call" }));

    const input = screen.getByLabelText("meeting title");
    fireEvent.change(input, { target: { value: "Vendor call — pricing" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() =>
      expect(mockRename).toHaveBeenCalledWith("m1", "Vendor call — pricing"),
    );
  });

  it("treats an emptied title as a cancel, not a rename to nothing", async () => {
    // A meeting with an empty title is unrecognisable in the library.
    render(<MeetingDocument meetingId="m1" onBack={() => {}} />);
    fireEvent.click(await screen.findByRole("heading", { name: "Vendor call" }));

    const input = screen.getByLabelText("meeting title");
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => expect(screen.queryByLabelText("meeting title")).toBeNull());
    expect(mockRename).not.toHaveBeenCalled();
  });

  it("abandons an edit on Escape", async () => {
    render(<MeetingDocument meetingId="m1" onBack={() => {}} />);
    fireEvent.click(await screen.findByRole("heading", { name: "Vendor call" }));

    const input = screen.getByLabelText("meeting title");
    fireEvent.change(input, { target: { value: "discarded" } });
    fireEvent.keyDown(input, { key: "Escape" });

    expect(mockRename).not.toHaveBeenCalled();
    expect(
      await screen.findByRole("heading", { name: "Vendor call" }),
    ).toBeInTheDocument();
  });

  it("says so when the meeting is gone rather than hanging on Loading", async () => {
    mockList.mockResolvedValue([]);
    render(<MeetingDocument meetingId="missing" onBack={() => {}} />);
    expect(await screen.findByText(/no longer in the database/)).toBeInTheDocument();
  });
});
