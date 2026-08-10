import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Onboarding, DISMISSED_KEY } from "./Onboarding";
import {
  meetingsList,
  onSidecarEvent,
  permissionsSnapshot,
  providerCurrent,
  providersList,
  runtimeState,
} from "../lib/tauri";

vi.mock("../lib/tauri", () => ({
  meetingsList: vi.fn(),
  onSidecarEvent: vi.fn(),
  openPrivacySettings: vi.fn(),
  permissionsSnapshot: vi.fn(),
  providerCurrent: vi.fn(),
  providersList: vi.fn(),
  runtimeState: vi.fn(),
  sidecarSend: vi.fn(),
  sidecarStart: vi.fn(),
}));
vi.mock("./ModelPicker", () => ({ ModelPicker: () => <div /> }));

const mockMeetings = vi.mocked(meetingsList);
const mockEvents = vi.mocked(onSidecarEvent);
const mockPermissions = vi.mocked(permissionsSnapshot);
const mockProvider = vi.mocked(providerCurrent);
const mockProviders = vi.mocked(providersList);
const mockRuntime = vi.mocked(runtimeState);

/** A machine that has never been set up: no permissions, no model. */
function unconfigured() {
  mockPermissions.mockResolvedValue({
    microphone: "undetermined",
    screenRecording: "undetermined",
    needsRelaunch: false,
  });
  mockProvider.mockResolvedValue({ kind: "Ollama" } as never);
  mockProviders.mockResolvedValue([]);
  mockRuntime.mockResolvedValue({ installedModels: [] } as never);
}

beforeEach(() => {
  for (const m of [
    mockMeetings,
    mockEvents,
    mockPermissions,
    mockProvider,
    mockProviders,
    mockRuntime,
  ]) {
    m.mockReset();
  }
  localStorage.removeItem(DISMISSED_KEY);
  mockEvents.mockResolvedValue(() => {});
  unconfigured();
  mockMeetings.mockResolvedValue([]);
});

describe("Onboarding", () => {
  it("shows nothing before it knows anything", async () => {
    // The bug this pins: `meetings: 0` meant both "no meetings" and "have not
    // checked yet", so returning to the library flashed the setup card for a
    // frame on a fully configured machine. Until the answer is in, the honest
    // render is nothing.
    mockMeetings.mockImplementation(() => new Promise(() => {}));
    render(<Onboarding />);
    expect(screen.queryByText(/Set up Oatmeal/)).toBeNull();
  });

  it("shows setup on a machine that genuinely needs it", async () => {
    render(<Onboarding />);
    expect(await screen.findByText(/Set up Oatmeal/)).toBeInTheDocument();
  });

  it("stays hidden once meetings exist", async () => {
    // Someone with six meetings has plainly finished setting up, whatever the
    // permission snapshot says at this instant.
    mockMeetings.mockResolvedValue([{ id: "m1" }] as never);
    render(<Onboarding />);
    await waitFor(() => expect(mockMeetings).toHaveBeenCalled());
    expect(screen.queryByText(/Set up Oatmeal/)).toBeNull();
  });

  it("does not flash even when the permission check fails", async () => {
    // A rejected snapshot must not be read as "no permissions, show setup".
    mockPermissions.mockRejectedValue("sidecar not up");
    mockMeetings.mockResolvedValue([{ id: "m1" }] as never);
    render(<Onboarding />);
    await waitFor(() => expect(mockMeetings).toHaveBeenCalled());
    expect(screen.queryByText(/Set up Oatmeal/)).toBeNull();
  });

  it("does not demand setup from someone with a library, when a probe fails", async () => {
    // The four questions used to share one try block with the meeting count
    // last. Anything failing above it skipped the count, left it at zero, and
    // zero meetings is exactly what makes this card appear — so an unreachable
    // runtime told a user of six months' standing to set the app up.
    mockRuntime.mockRejectedValue(new Error("runtime unreachable"));
    mockMeetings.mockResolvedValue([
      { id: "m1", title: "Vendor call" },
      { id: "m2", title: "Design review" },
    ] as never);

    render(<Onboarding />);
    await waitFor(() => expect(mockMeetings).toHaveBeenCalled());
    expect(screen.queryByText(/Set up Oatmeal/i)).toBeNull();
  });
});
