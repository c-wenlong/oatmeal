import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Library, dayKey, dayLabel, groupByDay, isLive, meetingTitle } from "./Library";
import { meetingsList } from "../lib/tauri";
import type { MeetingSummary } from "../types";

vi.mock("../lib/tauri", () => ({ meetingsList: vi.fn() }));
const mockList = vi.mocked(meetingsList);

/** 2026-08-07 14:30 local. */
const NOW = new Date(2026, 7, 7, 14, 30).getTime();
const DAY = 86_400_000;

function meeting(over: Partial<MeetingSummary> = {}): MeetingSummary {
  return {
    id: "m1",
    title: "Vendor call",
    startedAt: NOW,
    endedAt: NOW + 1800_000,
    status: "complete",
    audioPath: null,
    utteranceCount: 42,
    ...over,
  };
}

beforeEach(() => {
  mockList.mockReset();
  mockList.mockResolvedValue([meeting()]);
});

describe("dayKey", () => {
  it("uses the local day, not UTC", () => {
    // The bug this prevents: toISOString() converts to UTC, filing an early
    // morning meeting in Singapore under the previous day.
    const earlyMorning = new Date(2026, 7, 7, 0, 30).getTime();
    const lateEvening = new Date(2026, 7, 7, 23, 30).getTime();
    expect(dayKey(earlyMorning)).toBe("2026-08-07");
    expect(dayKey(lateEvening)).toBe("2026-08-07");
    expect(dayKey(earlyMorning)).toBe(dayKey(lateEvening));
  });

  it("pads months and days so keys sort correctly as strings", () => {
    // Groups are ordered by comparing these keys, so "2026-9-1" would sort
    // above "2026-10-1" and put September on top of October.
    expect(dayKey(new Date(2026, 0, 5).getTime())).toBe("2026-01-05");
  });
});

describe("dayLabel", () => {
  it("says Today and Yesterday relative to the given now", () => {
    expect(dayLabel(NOW, NOW)).toBe("Today");
    expect(dayLabel(NOW - DAY, NOW)).toBe("Yesterday");
  });

  it("falls back to a date further back", () => {
    const label = dayLabel(NOW - 20 * DAY, NOW);
    expect(label).not.toMatch(/Today|Yesterday/);
    expect(label).toMatch(/Jul/);
  });

  it("is relative to the passed clock, not the real one", () => {
    // If `now` were read internally this test would pass today and fail in a
    // day, which is the classic way a date test rots.
    const pretendNow = new Date(2027, 2, 15, 9, 0).getTime();
    expect(dayLabel(pretendNow, pretendNow)).toBe("Today");
    expect(dayLabel(NOW, pretendNow)).not.toBe("Today");
  });
});

describe("meetingTitle", () => {
  it("uses the title when there is one", () => {
    expect(meetingTitle(meeting({ title: "Vendor call" }))).toBe("Vendor call");
  });

  it("names untitled meetings by date rather than 'Untitled'", () => {
    // A list of six "Untitled meeting" rows is not a list.
    expect(meetingTitle(meeting({ title: null }))).toMatch(/August 7/);
  });

  it("treats a whitespace-only title as absent", () => {
    expect(meetingTitle(meeting({ title: "   " }))).toMatch(/August 7/);
  });
});

describe("groupByDay", () => {
  it("puts the newest day first", () => {
    const groups = groupByDay(
      [
        meeting({ id: "old", startedAt: NOW - 3 * DAY }),
        meeting({ id: "new", startedAt: NOW }),
      ],
      NOW,
    );
    expect(groups.map((g) => g.meetings[0].id)).toEqual(["new", "old"]);
  });

  it("puts the newest meeting first within a day", () => {
    const groups = groupByDay(
      [
        meeting({ id: "morning", startedAt: new Date(2026, 7, 7, 9).getTime() }),
        meeting({ id: "evening", startedAt: new Date(2026, 7, 7, 18).getTime() }),
      ],
      NOW,
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].meetings.map((m) => m.id)).toEqual(["evening", "morning"]);
  });

  it("keeps meetings on the same local day together", () => {
    const groups = groupByDay(
      [
        meeting({ id: "a", startedAt: new Date(2026, 7, 7, 0, 5).getTime() }),
        meeting({ id: "b", startedAt: new Date(2026, 7, 7, 23, 55).getTime() }),
      ],
      NOW,
    );
    expect(groups).toHaveLength(1);
  });

  it("does not merge different days", () => {
    const groups = groupByDay(
      [
        meeting({ id: "a", startedAt: NOW }),
        meeting({ id: "b", startedAt: NOW - DAY }),
      ],
      NOW,
    );
    expect(groups).toHaveLength(2);
  });

  it("handles an empty list without inventing a group", () => {
    expect(groupByDay([], NOW)).toEqual([]);
  });
});

describe("isLive", () => {
  it("counts recording and processing as live", () => {
    expect(isLive(meeting({ status: "recording" }))).toBe(true);
    expect(isLive(meeting({ status: "processing" }))).toBe(true);
  });

  it("does not count a finished meeting", () => {
    expect(isLive(meeting({ status: "complete" }))).toBe(false);
  });
});

describe("Library", () => {
  it("lists meetings under day headers", async () => {
    render(<Library onOpen={() => {}} now={NOW} />);
    expect(await screen.findByText("Vendor call")).toBeInTheDocument();
    expect(screen.getByText("Today")).toBeInTheDocument();
  });

  it("opens the meeting that was clicked", async () => {
    const onOpen = vi.fn();
    mockList.mockResolvedValue([
      meeting({ id: "wanted", title: "The one" }),
      meeting({ id: "other", title: "Not this" }),
    ]);
    render(<Library onOpen={onOpen} now={NOW} />);

    fireEvent.click(await screen.findByText("The one"));
    expect(onOpen).toHaveBeenCalledWith("wanted");
  });

  it("says so when there are no meetings, rather than showing nothing", async () => {
    mockList.mockResolvedValue([]);
    render(<Library onOpen={() => {}} now={NOW} />);
    expect(await screen.findByText(/No meetings yet/)).toBeInTheDocument();
  });

  it("marks a meeting that is still recording", async () => {
    mockList.mockResolvedValue([meeting({ status: "recording" })]);
    render(<Library onOpen={() => {}} now={NOW} />);
    expect(await screen.findByText("recording")).toBeInTheDocument();
  });

  it("surfaces a load failure instead of pretending the library is empty", async () => {
    // An empty library and a broken one look identical otherwise, and the user
    // would conclude their meetings were lost.
    mockList.mockRejectedValue("database is locked");
    render(<Library onOpen={() => {}} now={NOW} />);
    expect(await screen.findByText(/database is locked/)).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText(/No meetings yet/)).toBeNull());
  });
});
