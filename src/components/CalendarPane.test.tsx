import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CalendarPane, dotColor, emptyListNote } from "./CalendarPane";
import {
  calendarAccess,
  calendarResetVisibility,
  calendarSetDisplay,
  calendarSetVisible,
  calendarSources,
  detectionSettings,
} from "../lib/tauri";
import type { CalendarSource } from "../types";

vi.mock("../lib/tauri", () => ({
  detectionSettings: vi.fn(),
  calendarAccess: vi.fn(),
  calendarSources: vi.fn(),
  calendarSetVisible: vi.fn(),
  calendarResetVisibility: vi.fn(),
  calendarSetDisplay: vi.fn(),
}));

const mockSettings = vi.mocked(detectionSettings);
const mockSources = vi.mocked(calendarSources);
const mockAccess = vi.mocked(calendarAccess);
const mockSetVisible = vi.mocked(calendarSetVisible);
const mockReset = vi.mocked(calendarResetVisibility);
const mockSetDisplay = vi.mocked(calendarSetDisplay);

const source = (over: Partial<CalendarSource> = {}): CalendarSource => ({
  id: "work",
  title: "Work",
  color: "#3b82f6",
  visible: true,
  ...over,
});

beforeEach(() => {
  for (const m of [
    mockSettings,
    mockSources,
    mockSetVisible,
    mockReset,
    mockSetDisplay,
    mockAccess,
  ]) {
    m.mockReset();
  }
  mockSettings.mockResolvedValue({
    leadMs: 90_000,
    micEnabled: false,
    calendarEnabled: true,
    includeSoloEvents: false,
    showUpcomingInMenuBar: false,
  });
  mockSources.mockResolvedValue([
    source(),
    source({ id: "personal", title: "Personal" }),
  ]);
  mockSetVisible.mockResolvedValue(undefined);
  mockReset.mockResolvedValue(undefined);
  mockSetDisplay.mockResolvedValue(undefined);
  mockAccess.mockResolvedValue({ authorized: true, checkedAtMs: 1 });
});

describe("emptyListNote", () => {
  it("says detection is off rather than that you have no calendars", () => {
    // The list is empty for two very different reasons and only one of them is
    // the user's to fix.
    expect(emptyListNote(false, true, null)).toMatch(/detection is off/i);
  });

  it("distinguishes still-loading from genuinely none", () => {
    const granted = { authorized: true, checkedAtMs: 1 };
    expect(emptyListNote(true, false, granted)).toMatch(/Reading/i);
    expect(emptyListNote(true, true, granted)).toMatch(/Calendar app has/i);
  });

  it("says a Google account is a separate source", () => {
    // Someone whose calendars live only in Google will see this list empty
    // forever, and has no way to know why unless it is said.
    expect(emptyListNote(true, true, { authorized: true, checkedAtMs: 1 })).toMatch(
      /Google account is a separate source/i,
    );
  });

  it("names a refused permission rather than blaming the calendar", () => {
    // The app is told this and used to throw it away, so an empty list could
    // only be explained by guessing at it.
    expect(emptyListNote(true, true, { authorized: false, checkedAtMs: 1 })).toMatch(
      /System Settings/i,
    );
  });

  it("distinguishes a silent sidecar from a refused permission", () => {
    // Nothing reported is a different problem from something refused, and only
    // one of the two is the user's to fix.
    expect(emptyListNote(true, true, null)).toMatch(/sidecar has not reported/i);
  });
});

describe("dotColor", () => {
  it("falls back to the text colour rather than to nothing", () => {
    // A missing colour must not render an invisible dot and a ragged column.
    expect(dotColor({ color: null })).toBe("currentColor");
    expect(dotColor({ color: "#ff0000" })).toBe("#ff0000");
  });
});

describe("CalendarPane", () => {
  it("lists every calendar with a switch", async () => {
    render(<CalendarPane />);
    expect(await screen.findByRole("switch", { name: "Work" })).toBeChecked();
    expect(screen.getByRole("switch", { name: "Personal" })).toBeChecked();
  });

  it("switching a calendar off stores it", async () => {
    render(<CalendarPane />);
    fireEvent.click(await screen.findByRole("switch", { name: "Work" }));
    await waitFor(() => expect(mockSetVisible).toHaveBeenCalledWith("work", false));
  });

  it("shows a calendar the backend reports as hidden", async () => {
    // The switch reflects what is stored, not what was last clicked.
    mockSources.mockResolvedValue([source({ visible: false })]);
    render(<CalendarPane />);
    expect(await screen.findByRole("switch", { name: "Work" })).not.toBeChecked();
  });

  it("offers Reset only when something is switched off", async () => {
    render(<CalendarPane />);
    await screen.findByRole("switch", { name: "Work" });
    // A Reset that is always there is a button the user has to think about
    // every time they look at the page.
    expect(screen.queryByRole("button", { name: /reset/i })).toBeNull();

    mockSources.mockResolvedValue([source({ visible: false })]);
    render(<CalendarPane />);
    expect(await screen.findByRole("button", { name: /reset/i })).toBeInTheDocument();
  });

  it("Reset switches them all back on", async () => {
    mockSources.mockResolvedValue([source({ visible: false })]);
    render(<CalendarPane />);
    fireEvent.click(await screen.findByRole("button", { name: /reset/i }));
    await waitFor(() => expect(mockReset).toHaveBeenCalled());
  });

  it("wires both display switches to real settings", async () => {
    // Neither of these is decoration: one feeds the menu bar, the other the
    // rule that decides whether an entry is offered at all.
    render(<CalendarPane />);
    fireEvent.click(
      await screen.findByRole("switch", { name: /upcoming meetings in menu bar/i }),
    );
    await waitFor(() =>
      expect(mockSetDisplay).toHaveBeenCalledWith("showUpcomingInMenuBar", true),
    );

    fireEvent.click(
      screen.getByRole("switch", { name: /events with no participants/i }),
    );
    await waitFor(() =>
      expect(mockSetDisplay).toHaveBeenCalledWith("includeSoloEvents", true),
    );
  });

  it("reflects the stored display settings rather than defaulting to off", async () => {
    mockSettings.mockResolvedValue({
      leadMs: 90_000,
      micEnabled: false,
      calendarEnabled: true,
      includeSoloEvents: true,
      showUpcomingInMenuBar: true,
    });
    render(<CalendarPane />);
    expect(
      await screen.findByRole("switch", { name: /upcoming meetings in menu bar/i }),
    ).toBeChecked();
  });

  it("lists a connected Google account alongside the local calendars", async () => {
    // Its scope cannot enumerate calendars, so it is one row rather than
    // several — but leaving it out made the one source the user explicitly
    // connected the one source they could not find.
    mockSources.mockResolvedValue([
      source(),
      source({ id: "google:primary", title: "Google Calendar", visible: false }),
    ]);
    render(<CalendarPane />);
    expect(
      await screen.findByRole("switch", { name: "Google Calendar" }),
    ).not.toBeChecked();

    fireEvent.click(screen.getByRole("switch", { name: "Google Calendar" }));
    await waitFor(() =>
      expect(mockSetVisible).toHaveBeenCalledWith("google:primary", true),
    );
  });

  it("explains an empty list instead of showing nothing", async () => {
    mockSources.mockResolvedValue([]);
    render(<CalendarPane />);
    expect(await screen.findByText(/Calendar app has/i)).toBeInTheDocument();
  });
});

describe("advice when the calendar is not local", () => {
  it("does not send a Google user to macOS settings", () => {
    // Their calendar is not in Calendar.app and never will be. Telling them to
    // grant macOS access is advice that leads nowhere.
    expect(
      emptyListNote(true, true, { authorized: false, checkedAtMs: 1 }, true),
    ).toMatch(/Google account is connected/i);
  });

  it("still explains a refused permission when there is no account", () => {
    expect(
      emptyListNote(true, true, { authorized: false, checkedAtMs: 1 }, false),
    ).toMatch(/System Settings/i);
  });
});
