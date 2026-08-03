import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PrivacyCard, isLocal, provenanceLabel, retentionLabel } from "./PrivacyCard";
import { privacyPurgeAudio, privacySetRetention, privacySnapshot } from "../lib/tauri";
import type { PrivacySnapshot } from "../types";

vi.mock("../lib/tauri", () => ({
  privacySnapshot: vi.fn(),
  privacySetRetention: vi.fn(),
  privacyPurgeAudio: vi.fn(),
}));

const mockSnapshot = vi.mocked(privacySnapshot);
const mockSet = vi.mocked(privacySetRetention);
const mockPurge = vi.mocked(privacyPurgeAudio);

function snapshot(over: Partial<PrivacySnapshot> = {}): PrivacySnapshot {
  return {
    retention: { kind: "days", days: 7 },
    audioFiles: 3,
    audioBytes: 3_145_728,
    telemetry: false,
    generations: [
      {
        panelId: "p1",
        meetingId: "m1",
        meetingTitle: "Standup",
        provider: "ollama",
        model: "gemma4:e2b",
        generatedAt: 0,
        local: true,
      },
    ],
    ...over,
  };
}

beforeEach(() => {
  for (const m of [mockSnapshot, mockSet, mockPurge]) m.mockReset();
  mockSnapshot.mockResolvedValue(snapshot());
  mockSet.mockResolvedValue(undefined);
  mockPurge.mockResolvedValue({ deleted: 3, alreadyMissing: 0, freedBytes: 3_145_728 });
});

describe("isLocal", () => {
  it("reports the verdict Rust computed", () => {
    // Deliberately not re-deriving it from the provider string. `panels.provider`
    // stores a display label for older rows, and matching it here against
    // snake_case enum names reported every local summary as cloud.
    expect(isLocal({ local: true })).toBe(true);
    expect(isLocal({ local: false })).toBe(false);
  });
});

describe("retentionLabel", () => {
  it("reads naturally", () => {
    expect(retentionLabel({ kind: "days", days: 1 })).toBe("1 day");
    expect(retentionLabel({ kind: "days", days: 30 })).toBe("30 days");
    expect(retentionLabel({ kind: "forever" })).toBe("kept forever");
  });
});

describe("provenanceLabel", () => {
  it("names the provider and model", () => {
    expect(
      provenanceLabel({
        panelId: "p",
        meetingId: "m",
        meetingTitle: null,
        provider: "anthropic",
        model: "claude",
        generatedAt: 0,
        local: false,
      }),
    ).toBe("anthropic · claude");
  });

  it("does not invent a provider it does not know", () => {
    expect(
      provenanceLabel({
        panelId: "p",
        meetingId: "m",
        meetingTitle: null,
        provider: null,
        model: null,
        generatedAt: 0,
        local: false,
      }),
    ).toBe("unknown");
  });
});

describe("PrivacyCard", () => {
  it("says there is no telemetry", async () => {
    render(<PrivacyCard />);
    expect(await screen.findByText("no telemetry")).toBeInTheDocument();
  });

  it("shows what is on disk", async () => {
    render(<PrivacyCard />);
    expect(
      await screen.findByText(/3 audio files on disk, 3.0 MB/),
    ).toBeInTheDocument();
  });

  it("shows the current retention window as chosen", async () => {
    render(<PrivacyCard />);
    const chip = await screen.findByRole("button", { name: "7 days" });
    expect(chip.className).toContain("chip--on");
  });

  it("changes the retention window", async () => {
    render(<PrivacyCard />);
    fireEvent.click(await screen.findByRole("button", { name: "30 days" }));
    await waitFor(() =>
      expect(mockSet).toHaveBeenCalledWith({ kind: "days", days: 30 }),
    );
  });

  it("offers keep-forever", async () => {
    render(<PrivacyCard />);
    fireEvent.click(await screen.findByRole("button", { name: "kept forever" }));
    await waitFor(() => expect(mockSet).toHaveBeenCalledWith({ kind: "forever" }));
  });

  it("marks a local generation as on device", async () => {
    render(<PrivacyCard />);
    const list = await screen.findByTestId("provenance");
    expect(within(list).getByText(/on device · ollama/)).toBeInTheDocument();
  });

  it("marks a cloud generation and says the transcript was sent", async () => {
    // The honest answer someone actually needs: which summaries left.
    mockSnapshot.mockResolvedValue(
      snapshot({
        generations: [
          {
            panelId: "p1",
            meetingId: "m1",
            meetingTitle: "Board call",
            provider: "anthropic",
            model: "claude",
            generatedAt: 0,
            local: false,
          },
        ],
      }),
    );
    render(<PrivacyCard />);

    const list = await screen.findByTestId("provenance");
    expect(within(list).getByText(/cloud · anthropic/)).toBeInTheDocument();
    expect(
      await screen.findByText(/The transcript was sent to generate them/),
    ).toBeInTheDocument();
  });

  it("warns that purging keeps transcripts", async () => {
    // "Delete all audio" reads like it might take the meetings too.
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<PrivacyCard />);

    fireEvent.click(await screen.findByRole("button", { name: /delete all audio/i }));

    expect(confirm.mock.calls[0][0]).toMatch(
      /Transcripts, notes and summaries are kept/,
    );
    await waitFor(() => expect(mockPurge).toHaveBeenCalled());
    confirm.mockRestore();
  });

  it("does not purge when the confirmation is refused", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<PrivacyCard />);

    fireEvent.click(await screen.findByRole("button", { name: /delete all audio/i }));
    expect(mockPurge).not.toHaveBeenCalled();
    confirm.mockRestore();
  });

  it("reports what the purge freed", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<PrivacyCard />);

    fireEvent.click(await screen.findByRole("button", { name: /delete all audio/i }));
    expect(
      await screen.findByText(
        /Deleted 3 file\(s\), freeing 3.0 MB. Transcripts untouched./,
      ),
    ).toBeInTheDocument();
    confirm.mockRestore();
  });

  it("disables the purge button when there is nothing to delete", async () => {
    mockSnapshot.mockResolvedValue(snapshot({ audioFiles: 0, audioBytes: 0 }));
    render(<PrivacyCard />);
    expect(
      await screen.findByRole("button", { name: /delete all audio/i }),
    ).toBeDisabled();
  });
});
