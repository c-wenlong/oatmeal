import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderCard, modelAdvice, pullLabel } from "./ProviderCard";
import {
  onDownloadProgress,
  providerCurrent,
  providerModelAvailable,
  providerPullModel,
  providersList,
  runtimeState,
} from "../lib/tauri";

vi.mock("../lib/tauri", () => ({
  providersList: vi.fn(),
  providerCurrent: vi.fn(),
  providerSelect: vi.fn(),
  providerSetKey: vi.fn(),
  providerTest: vi.fn(),
  providerModelAvailable: vi.fn(),
  providerPullModel: vi.fn(),
  runtimeState: vi.fn(),
  onDownloadProgress: vi.fn(),
}));
vi.mock("./ModelPicker", () => ({ ModelPicker: () => <div /> }));

const mockProviders = vi.mocked(providersList);
const mockCurrent = vi.mocked(providerCurrent);
const mockAvailable = vi.mocked(providerModelAvailable);
const mockPull = vi.mocked(providerPullModel);
const mockRuntime = vi.mocked(runtimeState);
const mockProgress = vi.mocked(onDownloadProgress);

beforeEach(() => {
  for (const m of [mockProviders, mockCurrent, mockAvailable, mockPull, mockRuntime]) {
    m.mockReset();
  }
  mockProgress.mockReset();
  mockProgress.mockResolvedValue(() => {});
  mockProviders.mockResolvedValue([
    {
      kind: "ollama",
      label: "Ollama",
      defaultBaseUrl: "http://localhost:11434",
      defaultModel: "gemma4:e2b",
      requiresKey: false,
      isLocal: true,
      hasKey: false,
    },
  ]);
  mockCurrent.mockResolvedValue({
    id: "ollama",
    kind: "ollama",
    baseUrl: "http://localhost:11434",
    model: "gemma4:e2b",
    keychainRef: null,
  });
  mockAvailable.mockResolvedValue({ state: "installed", model: "gemma4:e2b" });
  mockPull.mockResolvedValue(undefined);
  mockRuntime.mockResolvedValue({ state: "ready" });
});

describe("modelAdvice", () => {
  it("says nothing when the model is there", () => {
    // A card that announces "installed" on every render is one more thing to
    // read on a page where nothing is happening.
    expect(modelAdvice({ state: "installed", model: "gemma4:e2b" })).toEqual({
      note: null,
      canPull: false,
    });
    expect(modelAdvice(null)).toEqual({ note: null, canPull: false });
  });

  it("offers a download only when one could work", () => {
    const missing = modelAdvice({ state: "missing", model: "gemma4:e2b" });
    expect(missing.canPull).toBe(true);
    expect(missing.note).toMatch(/gemma4:e2b is not installed/);
  });

  it("does not offer a download when nothing is listening", () => {
    // The button would fail against the same unreachable server, which is a
    // second confusing failure rather than a fix.
    const down = modelAdvice({
      state: "unreachable",
      detail: "could not reach Ollama",
    });
    expect(down.canPull).toBe(false);
    expect(down.note).toMatch(/could not reach Ollama/);
  });
});

describe("pullLabel", () => {
  it("counts megabytes when a total is known", () => {
    expect(pullLabel({ done: 52_428_800, total: 1_073_741_824 })).toBe(
      "Downloading 50/1024 MB",
    );
  });

  it("does not invent a percentage it cannot compute", () => {
    // Ollama reports each layer as its own run of bytes, so a share of the
    // whole would run backwards.
    expect(pullLabel({ done: 1, total: null })).toBe("Downloading…");
  });
});

describe("ProviderCard", () => {
  it("stays quiet when the model is installed", async () => {
    render(<ProviderCard />);
    await waitFor(() => expect(mockAvailable).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: /Download/ })).toBeNull();
  });

  it("offers to download a model that is not installed", async () => {
    mockAvailable.mockResolvedValue({ state: "missing", model: "gemma4:e2b" });
    render(<ProviderCard />);

    fireEvent.click(await screen.findByRole("button", { name: /Download gemma4:e2b/ }));
    await waitFor(() => expect(mockPull).toHaveBeenCalled());
  });

  it("explains an unreachable Ollama without offering a download", async () => {
    mockAvailable.mockResolvedValue({
      state: "unreachable",
      detail: "could not reach Ollama at http://localhost:11434",
    });
    render(<ProviderCard />);

    expect(await screen.findByText(/could not reach Ollama/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Download/ })).toBeNull();
  });

  it("does not fail the whole card when the check fails", async () => {
    // The availability probe is one question among several; a provider list
    // that loaded should still render.
    mockAvailable.mockRejectedValue(new Error("nope"));
    render(<ProviderCard />);
    expect(await screen.findByText("Summarisation model")).toBeInTheDocument();
  });
});
