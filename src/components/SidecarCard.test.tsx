import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SidecarCard } from "./SidecarCard";
import {
  onSidecarEvent,
  sidecarSend,
  sidecarSimulateCrash,
  sidecarStart,
  sidecarStop,
} from "../lib/tauri";
import type { SupervisorEvent } from "../types";

vi.mock("../lib/tauri", () => ({
  onSidecarEvent: vi.fn(),
  sidecarStart: vi.fn(),
  sidecarStop: vi.fn(),
  sidecarSend: vi.fn(),
  sidecarSimulateCrash: vi.fn(),
}));

const mockOn = vi.mocked(onSidecarEvent);
const mockStart = vi.mocked(sidecarStart);
const mockStop = vi.mocked(sidecarStop);
const mockSend = vi.mocked(sidecarSend);
const mockCrash = vi.mocked(sidecarSimulateCrash);

/** Captures the subscriber so tests can push events as the Rust side would. */
let subscriber: (event: SupervisorEvent) => void;

/**
 * Pushes an event the way the Rust side would. Wrapped in `act` because it
 * drives a React state update from outside the render cycle.
 */
function emit(event: SupervisorEvent) {
  act(() => subscriber(event));
}

// Block bodies throughout: the reset helpers return the mock, and vitest treats
// a function returned from a hook as a teardown callback — which would then call
// the mock after every test.
beforeEach(() => {
  mockOn.mockReset();
  mockStart.mockReset();
  mockStop.mockReset();
  mockSend.mockReset();
  mockCrash.mockReset();

  mockOn.mockImplementation(async (handler) => {
    subscriber = handler;
    return () => {};
  });
  mockStart.mockResolvedValue("/path/to/oatmeal-sidecar-aarch64-apple-darwin");
  mockStop.mockResolvedValue(undefined);
  mockSend.mockResolvedValue(undefined);
  mockCrash.mockResolvedValue(undefined);
});

describe("SidecarCard", () => {
  it("starts stopped and does not spawn anything on mount", () => {
    render(<SidecarCard />);
    expect(screen.getByText("stopped")).toBeInTheDocument();
    expect(mockStart).not.toHaveBeenCalled();
  });

  it("shows the binary path and goes green once the handshake lands", async () => {
    render(<SidecarCard />);
    await userEvent.click(screen.getByRole("button", { name: /start sidecar/i }));

    await waitFor(() => expect(mockStart).toHaveBeenCalled());
    expect(
      await screen.findByText(/oatmeal-sidecar-aarch64-apple-darwin/),
    ).toBeInTheDocument();

    // Still only "starting" until the sidecar actually announces itself.
    expect(screen.getByText("starting")).toBeInTheDocument();

    emit({ kind: "event", event: { ev: "ready", version: "0.1.0", protocol: 1 } });
    expect(await screen.findByText("connected")).toBeInTheDocument();
  });

  it("renders mic and system utterances with distinct tags", async () => {
    render(<SidecarCard />);
    await userEvent.click(screen.getByRole("button", { name: /start sidecar/i }));
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    emit({ kind: "event", event: { ev: "ready", version: "0.1.0", protocol: 1 } });
    emit({
      kind: "event",
      event: {
        ev: "final",
        source: "system",
        text: "So the deadline is the fourteenth.",
        t0: 400,
        t1: 3200,
        conf: 0.93,
      },
    });
    emit({
      kind: "event",
      event: {
        ev: "final",
        source: "mic",
        text: "Got it, I'll own the rollback plan.",
        t0: 3400,
        t1: 5600,
        conf: 0.88,
      },
    });

    expect(await screen.findByText(/deadline is the fourteenth/)).toBeInTheDocument();
    expect(screen.getByText(/rollback plan/)).toBeInTheDocument();
    expect(screen.getByText("mic")).toBeInTheDocument();
    expect(screen.getByText("system")).toBeInTheDocument();
  });

  it("shows a crash and the scheduled restart, then recovers on the next ready", async () => {
    render(<SidecarCard />);
    await userEvent.click(screen.getByRole("button", { name: /start sidecar/i }));
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    emit({ kind: "event", event: { ev: "ready", version: "0.1.0", protocol: 1 } });
    expect(await screen.findByText("connected")).toBeInTheDocument();

    emit({ kind: "exited", code: null, restarting_in_ms: 200 });
    expect(await screen.findByText(/restarting in 200ms/)).toBeInTheDocument();

    emit({ kind: "spawned", pid: 42, attempt: 2 });
    emit({ kind: "event", event: { ev: "ready", version: "0.1.0", protocol: 1 } });

    expect(await screen.findByText(/attempt 2/)).toBeInTheDocument();
    expect(screen.getByText("connected")).toBeInTheDocument();
  });

  it("goes red and stays red when the supervisor gives up", async () => {
    render(<SidecarCard />);
    await userEvent.click(screen.getByRole("button", { name: /start sidecar/i }));
    await waitFor(() => expect(mockOn).toHaveBeenCalled());

    emit({ kind: "event", event: { ev: "ready", version: "0.1.0", protocol: 1 } });
    emit({ kind: "gave_up", reason: "sidecar exited 5 times in a row" });

    expect(await screen.findByText("failed")).toBeInTheDocument();
    expect(screen.getByText(/exited 5 times in a row/)).toBeInTheDocument();
  });

  it("only enables session controls once connected", async () => {
    render(<SidecarCard />);
    expect(screen.getByRole("button", { name: /run session/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /simulate crash/i })).toBeDisabled();

    await userEvent.click(screen.getByRole("button", { name: /start sidecar/i }));
    await waitFor(() => expect(mockOn).toHaveBeenCalled());
    emit({ kind: "event", event: { ev: "ready", version: "0.1.0", protocol: 1 } });

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /run session/i })).toBeEnabled(),
    );
    await userEvent.click(screen.getByRole("button", { name: /run session/i }));
    expect(mockSend).toHaveBeenCalledWith({
      cmd: "start",
      meeting_id: "harness",
      sources: ["mic", "system"],
    });
  });

  it("surfaces a failure to start instead of appearing to work", async () => {
    mockStart.mockRejectedValue(new Error("sidecar binary not found"));
    render(<SidecarCard />);

    await userEvent.click(screen.getByRole("button", { name: /start sidecar/i }));

    expect(await screen.findByText(/sidecar binary not found/)).toBeInTheDocument();
    expect(screen.getByText("failed")).toBeInTheDocument();
  });
});
