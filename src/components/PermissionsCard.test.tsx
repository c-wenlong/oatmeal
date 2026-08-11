import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PermissionsCard, headlineTone } from "./PermissionsCard";
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

/** The card's own indicator tone — its state, without a badge to read. */
async function cardTone(): Promise<string> {
  const dot = await waitFor(() => {
    const found = document.querySelector(".card-head .perm-dot");
    if (!found) throw new Error("no indicator");
    return found;
  });
  return dot.className.replace(/.*perm-dot--/, "");
}

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

    expect(await cardTone()).toBe("ok");
  });

  it("prefers a live event over the seeded cache", async () => {
    mockSnapshot.mockResolvedValue({
      microphone: "granted",
      screenRecording: "granted",
      needsRelaunch: false,
    });
    render(<PermissionsCard />);
    expect(await cardTone()).toBe("ok");

    // Permission revoked while running: the fresher event must win.
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emitPermissions("denied", "granted");
    expect(await cardTone()).toBe("err");
  });

  it("stays usable when nothing is cached yet", async () => {
    mockSnapshot.mockResolvedValue(null);
    render(<PermissionsCard />);
    expect(await cardTone()).toBe("pending");
  });

  it("starts unknown and does not claim readiness before checking", async () => {
    render(<PermissionsCard />);
    expect(await cardTone()).toBe("pending");
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

    expect(await cardTone()).toBe("ok");
    expect(screen.getByText(/Ready to record/i)).toBeInTheDocument();
  });

  it("blocks capture when the mic is denied and offers the settings pane", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emitPermissions("denied", "granted");

    expect(await cardTone()).toBe("err");

    await userEvent.click(screen.getByRole("switch", { name: "Microphone" }));
    expect(mockOpen).toHaveBeenCalledWith("microphone");
  });

  it("deep-links to the screen recording pane, not a generic settings window", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emitPermissions("granted", "denied");

    // Screen Recording is offered the prompt first — CoreGraphics cannot say
    // whether it was ever asked — and only falls back to Settings once that
    // has visibly changed nothing.
    const toggle = await screen.findByRole("switch", { name: /Screen & System Audio/ });
    await userEvent.click(toggle);
    act(() => emitPermissions("granted", "denied"));

    await userEvent.click(
      screen.getByRole("switch", { name: /Screen & System Audio/ }),
    );
    expect(mockOpen).toHaveBeenCalledWith("screen_recording");
  });

  it("tells the user to relaunch when the grant is stale", async () => {
    render(<PermissionsCard />);
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    // Both granted, but the running process still holds the old denial.
    emitPermissions("granted", "granted", true);

    expect(await cardTone()).toBe("err");
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

describe("headlineTone", () => {
  it("keeps not-yet-known apart from blocked", () => {
    // One is the sidecar not having reported; the other is a problem the user
    // has to go and fix. Showing them the same colour asks them to act on a
    // question that has not been answered yet.
    expect(headlineTone(null, false)).toBe("pending");
    expect(
      headlineTone(
        { microphone: "granted", screenRecording: "denied", needsRelaunch: false },
        false,
      ),
    ).toBe("err");
    expect(
      headlineTone(
        { microphone: "granted", screenRecording: "granted", needsRelaunch: false },
        true,
      ),
    ).toBe("ok");
  });
});

describe("the state is not colour alone", () => {
  it("carries each state in the switch, not in colour", async () => {
    // A control that only differs by hue is unreadable to a colour-blind user
    // and invisible to a screen reader; `checked` is announced by both.
    render(<PermissionsCard />);
    act(() => emitPermissions("granted", "denied", false));

    expect(await screen.findByRole("switch", { name: "Microphone" })).toBeChecked();
    expect(
      screen.getByRole("switch", { name: /Screen & System Audio/ }),
    ).not.toBeChecked();
  });
});

describe("asking macOS from the row", () => {
  it("prompts for that permission alone, not both", async () => {
    // Requesting both fires two system dialogs one after the other, which is
    // startling from a switch sitting on one row.
    render(<PermissionsCard />);
    act(() => emitPermissions("undetermined", "granted", false));

    await userEvent.click(await screen.findByRole("switch", { name: "Microphone" }));
    expect(sidecarSend).toHaveBeenCalledWith({
      cmd: "permissions",
      request: true,
      pane: "microphone",
    });
  });

  it("sends to System Settings once macOS has recorded a denial", async () => {
    // The prompt never returns after a denial, so asking would do nothing
    // visible. The switch takes the user where the change is possible.
    render(<PermissionsCard />);
    act(() => emitPermissions("denied", "granted", false));

    await userEvent.click(await screen.findByRole("switch", { name: "Microphone" }));
    expect(mockOpen).toHaveBeenCalledWith("microphone");
    expect(sidecarSend).not.toHaveBeenCalledWith(
      expect.objectContaining({ request: true }),
    );
  });

  it("a granted permission is switched off in System Settings", async () => {
    // No API revokes an app's own grant, so "off" can only mean "take me
    // where it can be turned off". A switch that refused to move would be
    // worse than one that goes somewhere useful.
    render(<PermissionsCard />);
    act(() => emitPermissions("granted", "granted", false));

    const toggle = await screen.findByRole("switch", { name: "Microphone" });
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);
    expect(mockOpen).toHaveBeenCalledWith("microphone");
  });

  it("does not flip until macOS says so", async () => {
    // Flipping optimistically would show "on" after a cancelled dialog, which
    // is the app claiming a permission it does not have.
    render(<PermissionsCard />);
    act(() => emitPermissions("undetermined", "granted", false));

    const toggle = await screen.findByRole("switch", { name: "Microphone" });
    await userEvent.click(toggle);
    expect(screen.getByRole("switch", { name: "Microphone" })).not.toBeChecked();
  });

  it("falls back to Settings when the screen-recording prompt changed nothing", async () => {
    // CoreGraphics cannot say whether it was ever asked, so the app tries the
    // prompt once and learns from the result rather than guessing.
    render(<PermissionsCard />);
    act(() => emitPermissions("granted", "denied", false));

    const toggle = await screen.findByRole("switch", { name: /Screen & System Audio/ });
    await userEvent.click(toggle);
    act(() => emitPermissions("granted", "denied", false));

    await userEvent.click(
      screen.getByRole("switch", { name: /Screen & System Audio/ }),
    );
    expect(mockOpen).toHaveBeenCalledWith("screen_recording");
  });

  it("offers the prompt for screen recording before anything is known", async () => {
    render(<PermissionsCard />);
    act(() => emitPermissions("granted", "denied", false));

    await userEvent.click(
      await screen.findByRole("switch", { name: /Screen & System Audio/ }),
    );
    expect(sidecarSend).toHaveBeenCalledWith({
      cmd: "permissions",
      request: true,
      pane: "screen_recording",
    });
  });
});
