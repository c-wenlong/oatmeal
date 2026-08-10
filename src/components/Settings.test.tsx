import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Settings, detectionSummary, googleSummary, paneTitle } from "./Settings";
import { PANES, navGroups, type Pane } from "./SettingsNav";
import { detectionSettings, gcalSettings } from "../lib/tauri";

vi.mock("../lib/tauri", () => ({
  detectionSettings: vi.fn(),
  gcalSettings: vi.fn(),
}));

// The rows wrap the real cards, every one of which reaches for Tauri. These
// tests are about which pane shows what, not about any card's behaviour — so
// each stands in as a marker that can be looked for by name.
vi.mock("./DetectionSettings", () => ({
  DetectionSettingsPanel: () => <div data-testid="detection-panel" />,
}));
vi.mock("./GoogleCalendarCard", () => ({
  GoogleCalendarCard: () => <div data-testid="card-calendar" />,
}));
vi.mock("./CalendarPane", () => ({
  CalendarPane: () => <div data-testid="calendar-pane" />,
}));
vi.mock("./NotionCard", () => ({
  NotionCard: () => <div data-testid="card-notion" />,
}));
vi.mock("./PermissionsCard", () => ({
  PermissionsCard: () => <div data-testid="card-permissions" />,
}));
vi.mock("./PrivacyCard", () => ({
  PrivacyCard: () => <div data-testid="card-privacy" />,
}));
vi.mock("./ProviderCard", () => ({
  ProviderCard: () => <div data-testid="card-provider" />,
}));
vi.mock("./UpdateCard", () => ({
  UpdateCard: () => <div data-testid="card-update" />,
}));

/** Everything the old single page rendered. Nothing may be lost in the move. */
const EVERY_CARD = [
  "card-calendar",
  "card-notion",
  "card-permissions",
  "card-privacy",
  "card-provider",
  "card-update",
];

const mockDetection = vi.mocked(detectionSettings);
const mockGcal = vi.mocked(gcalSettings);

beforeEach(() => {
  mockDetection.mockReset();
  mockGcal.mockReset();
  mockGcal.mockResolvedValue({
    connected: false,
    clientId: null,
    hasClientSecret: false,
    enabled: false,
  });
  mockDetection.mockResolvedValue({
    leadMs: 90_000,
    micEnabled: true,
    calendarEnabled: false,
    includeSoloEvents: false,
    showUpcomingInMenuBar: false,
  });
});

/** Clicks a pane in the sidebar by its name. */
function open(label: string) {
  fireEvent.click(screen.getByRole("button", { name: new RegExp(`^${label}$`) }));
}

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

describe("navGroups", () => {
  const pane = (id: string, group: string | null): Pane =>
    ({ id, label: id, icon: "info", group }) as Pane;

  it("keeps consecutive panes under one heading", () => {
    const groups = navGroups([pane("a", null), pane("b", "Data"), pane("c", "Data")]);
    expect(groups.map((g) => [g.label, g.panes.length])).toEqual([
      [null, 1],
      ["Data", 2],
    ]);
  });

  it("does not reorder a pane into an earlier group of the same name", () => {
    // Bucketing by label would silently move "c" up next to "a", so the sidebar
    // would stop matching the order the list is written in.
    const groups = navGroups([pane("a", "Data"), pane("b", null), pane("c", "Data")]);
    expect(groups.map((g) => g.panes.map((p) => p.id))).toEqual([["a"], ["b"], ["c"]]);
  });
});

describe("Settings", () => {
  it("shows one pane at a time, not the whole page", async () => {
    render(<Settings onBack={() => {}} />);
    expect(await screen.findByTestId("card-permissions")).toBeInTheDocument();
    expect(screen.queryByTestId("card-notion")).toBeNull();
  });

  it("moves to the pane the sidebar names", async () => {
    render(<Settings onBack={() => {}} />);
    open("Sharing");
    expect(await screen.findByTestId("card-notion")).toBeInTheDocument();
    expect(screen.queryByTestId("card-permissions")).toBeNull();
  });

  it("leaves every card reachable from the sidebar", async () => {
    // The risk in splitting one page into seven panes is a card that simply
    // stops being rendered anywhere: no error, no test failure, just a setting
    // that quietly no longer exists.
    render(<Settings onBack={() => {}} />);
    const seen = new Set<string>();
    for (const pane of PANES) {
      open(pane.label);
      for (const id of EVERY_CARD) {
        if (screen.queryByTestId(id)) seen.add(id);
      }
      // Cards behind a disclosure are still reachable; the walk has to open
      // them or the test would call a hidden card "lost".
      for (const row of screen.queryAllByRole("button")) {
        if (/Google Calendar|Meeting detection/.test(row.textContent ?? "")) {
          fireEvent.click(row);
          for (const id of EVERY_CARD) {
            if (screen.queryByTestId(id)) seen.add(id);
          }
          open(pane.label);
        }
      }
    }
    expect([...seen].sort()).toEqual([...EVERY_CARD].sort());
    // The Detection pane left a fetch in flight; settle it inside the test.
    open("Detection");
    expect(await screen.findByText("apps")).toBeInTheDocument();
  });

  it("marks where you are, and not merely with a colour", async () => {
    render(<Settings onBack={() => {}} />);
    open("Models");
    expect(screen.getByRole("button", { name: /^Models$/ })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("button", { name: /^Capture$/ })).not.toHaveAttribute(
      "aria-current",
    );
  });

  it("titles the pane with its own name", () => {
    expect(PANES.map((pane) => paneTitle(pane.id))).toEqual(PANES.map((p) => p.label));
  });

  it("shows detection as one row, not an expanded panel", async () => {
    // Inline, detection is two toggles, a slider and a thirteen-app list —
    // one row dwarfing every other row on the page.
    render(<Settings onBack={() => {}} />);
    open("Detection");
    expect(await screen.findByText("Meeting detection")).toBeInTheDocument();
    expect(screen.queryByTestId("detection-panel")).toBeNull();
  });

  it("summarises the current state on the row", async () => {
    render(<Settings onBack={() => {}} />);
    open("Detection");
    expect(await screen.findByText("apps")).toBeInTheDocument();
  });

  it("opens the detail when the row is pressed", async () => {
    render(<Settings onBack={() => {}} />);
    open("Detection");
    fireEvent.click(await screen.findByText("Meeting detection"));
    // findBy rather than getBy: opening the detail re-reads the settings, and
    // that resolves after this line would otherwise have run.
    expect(await screen.findByTestId("detection-panel")).toBeInTheDocument();
  });

  it("comes back to the detection pane, not out of settings entirely", async () => {
    // Back from a sub-screen must land one level up. Leaving settings would
    // lose the user's place for no reason.
    const onBack = vi.fn();
    render(<Settings onBack={onBack} />);
    open("Detection");
    fireEvent.click(await screen.findByText("Meeting detection"));
    fireEvent.click(screen.getByRole("button", { name: /‹ Detection/ }));

    expect(onBack).not.toHaveBeenCalled();
    expect(await screen.findByText("Meeting detection")).toBeInTheDocument();
    expect(screen.queryByTestId("detection-panel")).toBeNull();
  });

  it("does not keep a sub-screen open behind another pane", async () => {
    // Come back to Detection from Models and you should see Detection, not the
    // detail screen you left open several panes ago.
    render(<Settings onBack={() => {}} />);
    open("Detection");
    fireEvent.click(await screen.findByText("Meeting detection"));
    open("Models");
    open("Detection");

    expect(await screen.findByText("apps")).toBeInTheDocument();
    expect(screen.queryByTestId("detection-panel")).toBeNull();
  });

  it("leaves settings from the sidebar", () => {
    const onBack = vi.fn();
    render(<Settings onBack={onBack} />);
    fireEvent.click(screen.getByRole("button", { name: /‹ Meetings/ }));
    expect(onBack).toHaveBeenCalled();
  });
});

describe("googleSummary", () => {
  it("distinguishes never-set-up from set-up-but-not-connected", () => {
    // The two need different things from the user: one needs a credential,
    // the other needs them to press Connect.
    expect(googleSummary({ connected: false, clientId: null })).toBe("Not set up");
    expect(googleSummary({ connected: false, clientId: "x" })).toBe("Not connected");
    expect(googleSummary({ connected: true, clientId: "x" })).toBe("Connected");
  });

  it("does not claim a state before it knows one", () => {
    expect(googleSummary(null)).toBe("—");
  });
});
