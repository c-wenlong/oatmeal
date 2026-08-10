import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DetectionSettingsPanel, formatLead } from "./DetectionSettings";
import {
  detectionBuiltinApps,
  detectionRuleClear,
  detectionRulesList,
  detectionSetSettings,
  detectionSettings,
} from "../lib/tauri";

vi.mock("../lib/tauri", () => ({
  detectionSettings: vi.fn(),
  detectionSetSettings: vi.fn(),
  detectionRulesList: vi.fn(),
  detectionRuleClear: vi.fn(),
  detectionBuiltinApps: vi.fn(),
}));

const mockGet = vi.mocked(detectionSettings);
const mockSet = vi.mocked(detectionSetSettings);
const mockRules = vi.mocked(detectionRulesList);
const mockClear = vi.mocked(detectionRuleClear);
const mockBuiltins = vi.mocked(detectionBuiltinApps);

beforeEach(() => {
  for (const m of [mockGet, mockSet, mockRules, mockClear, mockBuiltins]) {
    m.mockReset();
  }
  mockGet.mockResolvedValue({
    leadMs: 90_000,
    micEnabled: false,
    calendarEnabled: false,
    includeSoloEvents: false,
    showUpcomingInMenuBar: false,
  });
  mockSet.mockResolvedValue(undefined);
  mockRules.mockResolvedValue([]);
  mockClear.mockResolvedValue(undefined);
  mockBuiltins.mockResolvedValue([["us.zoom.xos", "Zoom"]]);
});

describe("formatLead", () => {
  it("reads the way a person would say it", () => {
    expect(formatLead(90_000)).toBe("1 min 30s");
    expect(formatLead(120_000)).toBe("2 min");
    expect(formatLead(30_000)).toBe("30s");
    expect(formatLead(0)).toBe("0s");
  });
});

describe("DetectionSettingsPanel", () => {
  it("starts with both triggers off", async () => {
    // Detection watches other apps and reads the calendar. Neither should begin
    // because the app was launched.
    render(<DetectionSettingsPanel />);
    const mic = await screen.findByLabelText(/watch for calls in other apps/i);
    const calendar = screen.getByLabelText(/use my calendar/i);
    expect(mic).not.toBeChecked();
    expect(calendar).not.toBeChecked();
  });

  it("says plainly that it never records on its own", async () => {
    render(<DetectionSettingsPanel />);
    expect(await screen.findByText(/never starts on its own/i)).toBeInTheDocument();
  });

  it("saves a trigger toggle", async () => {
    render(<DetectionSettingsPanel />);
    fireEvent.click(await screen.findByLabelText(/watch for calls in other apps/i));

    await waitFor(() =>
      expect(mockSet).toHaveBeenCalledWith(
        expect.objectContaining({ micEnabled: true }),
      ),
    );
  });

  it("saves a new lead time", async () => {
    mockGet.mockResolvedValue({
      leadMs: 90_000,
      micEnabled: false,
      calendarEnabled: true,
      includeSoloEvents: false,
      showUpcomingInMenuBar: false,
    });
    render(<DetectionSettingsPanel />);

    const slider = await screen.findByLabelText(/calendar lead time/i);
    fireEvent.change(slider, { target: { value: "180000" } });

    await waitFor(() =>
      expect(mockSet).toHaveBeenCalledWith(
        expect.objectContaining({ leadMs: 180_000 }),
      ),
    );
  });

  it("disables the lead time when the calendar is off", async () => {
    render(<DetectionSettingsPanel />);
    expect(await screen.findByLabelText(/calendar lead time/i)).toBeDisabled();
  });

  it("shows the shipped defaults as allowed", async () => {
    render(<DetectionSettingsPanel />);
    const allowed = await screen.findByTestId("allowed-apps");
    expect(within(allowed).getByText(/Zoom/)).toBeInTheDocument();
    expect(within(allowed).getByText(/default/)).toBeInTheDocument();
  });

  it("shows a user rule instead of the default it overrides", async () => {
    // Otherwise Zoom appears twice saying opposite things.
    mockRules.mockResolvedValue([
      { bundleId: "us.zoom.xos", appName: "Zoom", mode: "ignore" },
    ]);
    render(<DetectionSettingsPanel />);

    const ignored = await screen.findByTestId("ignored-apps");
    expect(within(ignored).getByText("Zoom")).toBeInTheDocument();

    const allowed = screen.getByTestId("allowed-apps");
    expect(within(allowed).queryByText(/Zoom/)).toBeNull();
  });

  it("can reset a rule back to being asked about", async () => {
    mockRules.mockResolvedValue([
      { bundleId: "com.wisprflow.app", appName: "Wispr Flow", mode: "ignore" },
    ]);
    render(<DetectionSettingsPanel />);

    fireEvent.click(await screen.findByRole("button", { name: /reset/i }));
    await waitFor(() => expect(mockClear).toHaveBeenCalledWith("com.wisprflow.app"));
  });

  it("explains what happens to an app with no rule", async () => {
    render(<DetectionSettingsPanel />);
    expect(
      await screen.findByText(/asked about once, the first time/i),
    ).toBeInTheDocument();
  });
});
