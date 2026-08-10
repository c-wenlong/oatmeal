import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Settings, detectionSummary } from "./Settings";
import { detectionSettings } from "../lib/tauri";

vi.mock("../lib/tauri", () => ({ detectionSettings: vi.fn() }));

// The rows wrap the real cards, every one of which reaches for Tauri. This
// test is about which screen shows what, not about any card's behaviour.
for (const name of [
  "DetectionSettings",
  "GoogleCalendarCard",
  "NotionCard",
  "PermissionsCard",
  "PrivacyCard",
  "ProviderCard",
  "UpdateCard",
]) {
  vi.doMock(`./${name}`, () => ({}));
}
vi.mock("./DetectionSettings", () => ({
  DetectionSettingsPanel: () => <div data-testid="detection-panel" />,
}));
vi.mock("./GoogleCalendarCard", () => ({ GoogleCalendarCard: () => <div /> }));
vi.mock("./NotionCard", () => ({ NotionCard: () => <div /> }));
vi.mock("./PermissionsCard", () => ({ PermissionsCard: () => <div /> }));
vi.mock("./PrivacyCard", () => ({ PrivacyCard: () => <div /> }));
vi.mock("./ProviderCard", () => ({ ProviderCard: () => <div /> }));
vi.mock("./UpdateCard", () => ({ UpdateCard: () => <div /> }));

const mockDetection = vi.mocked(detectionSettings);

beforeEach(() => {
  mockDetection.mockReset();
  mockDetection.mockResolvedValue({
    leadMs: 90_000,
    micEnabled: true,
    calendarEnabled: false,
  });
});

describe("detectionSummary", () => {
  it("names what is on", () => {
    expect(detectionSummary({ micEnabled: true, calendarEnabled: true })).toBe(
      "apps and calendar",
    );
    expect(detectionSummary({ micEnabled: true, calendarEnabled: false })).toBe("apps");
  });

  it("says Off rather than nothing", () => {
    // A blank value in the row reads as still loading.
    expect(detectionSummary({ micEnabled: false, calendarEnabled: false })).toBe("Off");
  });

  it("does not claim a state before it knows one", () => {
    expect(detectionSummary(null)).toBe("—");
  });
});

describe("Settings", () => {
  it("shows detection as one row, not an expanded panel", async () => {
    // Inline, detection is two toggles, a slider and a thirteen-app list —
    // one row dwarfing every other row on the page.
    render(<Settings onBack={() => {}} />);
    expect(await screen.findByText("Meeting detection")).toBeInTheDocument();
    expect(screen.queryByTestId("detection-panel")).toBeNull();
  });

  it("summarises the current state on the row", async () => {
    render(<Settings onBack={() => {}} />);
    expect(await screen.findByText("apps")).toBeInTheDocument();
  });

  it("opens the detail when the row is pressed", async () => {
    render(<Settings onBack={() => {}} />);
    fireEvent.click(await screen.findByText("Meeting detection"));
    expect(screen.getByTestId("detection-panel")).toBeInTheDocument();
  });

  it("comes back to Preferences, not out of settings entirely", async () => {
    // Back from a sub-screen must land one level up. Leaving settings would
    // lose the user's place for no reason.
    const onBack = vi.fn();
    render(<Settings onBack={onBack} />);
    fireEvent.click(await screen.findByText("Meeting detection"));
    fireEvent.click(screen.getByRole("button", { name: /Preferences/ }));

    expect(onBack).not.toHaveBeenCalled();
    expect(await screen.findByText("Meeting detection")).toBeInTheDocument();
  });
});
