import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UpdateCard, isWorthReporting, summarise } from "./UpdateCard";
import { updateCheck, updateInstall, updateSkip } from "../lib/tauri";
import type { UpdateStatus } from "../types";

vi.mock("../lib/tauri", () => ({
  updateCheck: vi.fn(),
  updateInstall: vi.fn(),
  updateSkip: vi.fn(),
}));

const mockCheck = vi.mocked(updateCheck);
const mockInstall = vi.mocked(updateInstall);
const mockSkip = vi.mocked(updateSkip);

function status(over: Partial<UpdateStatus> = {}): UpdateStatus {
  return {
    currentVersion: "0.1.0",
    availableVersion: null,
    notes: null,
    decision: "up_to_date",
    ...over,
  };
}

const OFFERED = status({ availableVersion: "0.2.0", decision: "offer" });

beforeEach(() => {
  for (const m of [mockCheck, mockInstall, mockSkip]) m.mockReset();
  mockCheck.mockResolvedValue(status());
  mockInstall.mockResolvedValue(undefined);
  mockSkip.mockResolvedValue(undefined);
});

describe("summarise", () => {
  it("names the available version", () => {
    expect(summarise(OFFERED)).toMatch(/0\.2\.0 is available/);
  });

  it("says so when up to date, with the running version", () => {
    expect(summarise(status())).toMatch(/0\.1\.0 is up to date/);
  });

  it("remembers that a skipped version is still available", () => {
    // Otherwise skipping looks like the update vanished.
    expect(
      summarise(status({ availableVersion: "0.2.0", decision: "skipped" })),
    ).toMatch(/skip it/);
  });
});

describe("isWorthReporting", () => {
  it("stays quiet about being offline", () => {
    // A laptop on a train fails every check. That is not an error worth a card.
    expect(isWorthReporting("error sending request for url")).toBe(false);
  });

  it("speaks up when the updater is misconfigured", () => {
    // This one can never fix itself, and silence would hide it until a
    // security fix needed shipping.
    expect(isWorthReporting("the updater is not configured: missing pubkey")).toBe(
      true,
    );
  });
});

describe("UpdateCard", () => {
  it("checks on mount without being asked", async () => {
    render(<UpdateCard />);
    await waitFor(() => expect(mockCheck).toHaveBeenCalled());
  });

  it("announces an available version", async () => {
    mockCheck.mockResolvedValue(OFFERED);
    render(<UpdateCard />);
    expect(await screen.findByText(/0\.2\.0 is available/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /install and restart/i })).toBeEnabled();
  });

  it("shows release notes when there are any", async () => {
    mockCheck.mockResolvedValue({ ...OFFERED, notes: "Fixes the linker." });
    render(<UpdateCard />);
    expect(await screen.findByText(/Fixes the linker/)).toBeInTheDocument();
  });

  it("never installs without a click", async () => {
    // An app that swapped itself out mid-meeting would lose a recording.
    mockCheck.mockResolvedValue(OFFERED);
    render(<UpdateCard />);
    await screen.findByText(/0\.2\.0 is available/);
    expect(mockInstall).not.toHaveBeenCalled();
  });

  it("installs when asked", async () => {
    mockCheck.mockResolvedValue(OFFERED);
    render(<UpdateCard />);
    fireEvent.click(
      await screen.findByRole("button", { name: /install and restart/i }),
    );
    await waitFor(() => expect(mockInstall).toHaveBeenCalled());
  });

  it("warns that it will restart", async () => {
    mockCheck.mockResolvedValue(OFFERED);
    mockInstall.mockImplementation(() => new Promise(() => {}));
    render(<UpdateCard />);
    fireEvent.click(
      await screen.findByRole("button", { name: /install and restart/i }),
    );
    expect(await screen.findByText(/will restart/i)).toBeInTheDocument();
  });

  it("skips the exact version on offer", async () => {
    mockCheck.mockResolvedValue(OFFERED);
    render(<UpdateCard />);
    fireEvent.click(await screen.findByRole("button", { name: /skip this version/i }));
    await waitFor(() => expect(mockSkip).toHaveBeenCalledWith("0.2.0"));
  });

  it("offers no skip button once it is already skipped", async () => {
    mockCheck.mockResolvedValue(
      status({ availableVersion: "0.2.0", decision: "skipped" }),
    );
    render(<UpdateCard />);
    await screen.findByText(/skip it/);
    expect(screen.queryByRole("button", { name: /skip this version/i })).toBeNull();
    // But installing it anyway stays possible — skipping is not a refusal.
    expect(
      screen.getByRole("button", { name: /install and restart/i }),
    ).toBeInTheDocument();
  });

  it("stays silent when the check fails on its own", async () => {
    mockCheck.mockRejectedValue("error sending request for url");
    render(<UpdateCard />);
    await waitFor(() => expect(mockCheck).toHaveBeenCalled());
    expect(screen.queryByText(/error sending request/)).toBeNull();
  });

  it("reports a failure the user actually pressed for", async () => {
    // Silence after a button press reads as a broken button.
    mockCheck.mockRejectedValue("error sending request for url");
    render(<UpdateCard />);
    fireEvent.click(await screen.findByRole("button", { name: /check for updates/i }));
    expect(await screen.findByText(/error sending request/)).toBeInTheDocument();
  });

  it("reports a broken configuration even unprompted", async () => {
    mockCheck.mockRejectedValue("the updater is not configured: missing pubkey");
    render(<UpdateCard />);
    expect(await screen.findByText(/not configured/)).toBeInTheDocument();
  });
});
