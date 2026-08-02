import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PermissionsCard } from "./PermissionsCard";
import {
  onSidecarEvent,
  openPrivacySettings,
  permissionsSnapshot,
  sidecarSend,
} from "../lib/tauri";
import type { PermissionState, SupervisorEvent } from "../types";

vi.mock("../lib/tauri", () => ({
  onSidecarEvent: vi.fn(),
  sidecarSend: vi.fn(),
  openPrivacySettings: vi.fn(),
  permissionsSnapshot: vi.fn(),
}));

const mockOn = vi.mocked(onSidecarEvent);
const mockSend = vi.mocked(sidecarSend);
const mockOpen = vi.mocked(openPrivacySettings);
const mockSnapshot = vi.mocked(permissionsSnapshot);

let subscriber: (event: SupervisorEvent) => void;

function emitPermissions(
  microphone: PermissionState,
  screen_recording: PermissionState,
  needs_relaunch = false,
) {
  act(() =>
    subscriber({
      kind: "event",
      event: { ev: "permissions", microphone, screen_recording, needs_relaunch },
    }),
  );
}

// Block bodies: the reset helpers return the mock, and vitest treats a function
// returned from a hook as a teardown callback.
beforeEach(() => {
  mockOn.mockReset();
  mockSend.mockReset();
  mockOpen.mockReset();
  mockSnapshot.mockReset();
  // Default: nothing cached yet.
  mockSnapshot.mockResolvedValue(null);

  mockOn.mockImplementation(async (handler) => {
    subscriber = handler;
    return () => {};
  });
  mockSend.mockResolvedValue(undefined);
  mockOpen.mockResolvedValue(undefined);
});

describe("PermissionsCard", () => {
  it("seeds from the cached snapshot when it mounts after the sidecar reported", async () => {
    // Regression: permissions arrive as a single event. A card that only
    // subscribes to future events showed "unknown" forever on any later mount
    // (hot reload, second window, slow render) despite the answer being known.
    mockSnapshot.mockResolvedValue({
      microphone: "granted",
      screenRecording: "granted",
      needsRelaunch: false,
    });

    render(<PermissionsCard />);

    expect(await screen.findByText("ready")).toBeInTheDocument();
  });

  it("prefers a live event over the seeded cache", async () => {
    mockSnapshot.mockResolvedValue({
      microphone: "granted",
      screenRecording: "granted",
      needsRelaunch: false,
    });
    render(<PermissionsCard />);
    expect(await screen.findByText("ready")).toBeInTheDocument();

    // Permission revoked while running: the fresher event must win.
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emitPermissions("denied", "granted");
    expect(await screen.findByText("capture blocked")).toBeInTheDocument();
  });

  it("stays usable when nothing is cached yet", async () => {
    mockSnapshot.mockResolvedValue(null);
    render(<PermissionsCard />);
    expect(await screen.findByText("unknown")).toBeInTheDocument();
  });

  it("starts unknown and does not claim readiness before checking", async () => {
    render(<PermissionsCard />);
    expect(screen.getByText("unknown")).toBeInTheDocument();
    expect(screen.queryByText("ready")).not.toBeInTheDocument();
  });

  it("queries without prompting when Check is pressed", async () => {
    render(<PermissionsCard />);
    await userEvent.click(screen.getByRole("button", { name: /check permissions/i }));
    // A bare check must not fire system dialogs at the user.
    expect(mockSend).toHaveBeenCalledWith({ cmd: "permissions", request: false });
  });

  it("reports ready when both are granted", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emitPermissions("granted", "granted");

    expect(await screen.findByText("ready")).toBeInTheDocument();
    expect(screen.getByText(/Ready to record/i)).toBeInTheDocument();
  });

  it("blocks capture when the mic is denied and offers the settings pane", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emitPermissions("denied", "granted");

    expect(await screen.findByText("capture blocked")).toBeInTheDocument();

    await userEvent.click(screen.getAllByRole("button", { name: /open settings/i })[0]);
    expect(mockOpen).toHaveBeenCalledWith("microphone");
  });

  it("deep-links to the screen recording pane, not a generic settings window", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emitPermissions("granted", "denied");

    await userEvent.click(
      await screen.findByRole("button", { name: /open settings/i }),
    );
    expect(mockOpen).toHaveBeenCalledWith("screen_recording");
  });

  it("tells the user to relaunch when the grant is stale", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    // Both granted, but the running process still holds the old denial.
    emitPermissions("granted", "granted", true);

    expect(await screen.findByText("capture blocked")).toBeInTheDocument();
    expect(screen.getByText(/relaunch/i)).toBeInTheDocument();
  });

  it("enables Request only while a prompt could still work", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    emitPermissions("undetermined", "granted");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /request access/i })).toBeEnabled(),
    );

    // Once denied, macOS never shows the prompt again.
    emitPermissions("denied", "granted");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /request access/i })).toBeDisabled(),
    );
  });

  it("passes request:true only when explicitly asking", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emitPermissions("undetermined", "undetermined");

    await userEvent.click(screen.getByRole("button", { name: /request access/i }));
    expect(mockSend).toHaveBeenCalledWith({ cmd: "permissions", request: true });
  });

  it("surfaces a failure instead of silently doing nothing", async () => {
    mockSend.mockRejectedValue(new Error("sidecar is not running"));
    render(<PermissionsCard />);

    await userEvent.click(screen.getByRole("button", { name: /check permissions/i }));
    expect(await screen.findByText(/sidecar is not running/)).toBeInTheDocument();
  });
});
