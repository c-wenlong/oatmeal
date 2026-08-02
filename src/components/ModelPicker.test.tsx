import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ModelPicker, downloadLabel, formatBytes } from "./ModelPicker";
import {
  onDownloadProgress,
  runtimeCancelDownload,
  runtimeInstallModel,
  runtimeInstallServer,
  runtimeModelStatus,
  runtimeModels,
  runtimeState,
} from "../lib/tauri";
import type { DownloadProgress } from "../types";

vi.mock("../lib/tauri", () => ({
  runtimeState: vi.fn(),
  runtimeModels: vi.fn(),
  runtimeModelStatus: vi.fn(),
  runtimeInstallServer: vi.fn(),
  runtimeInstallModel: vi.fn(),
  runtimeCancelDownload: vi.fn(),
  onDownloadProgress: vi.fn(),
}));

const mockState = vi.mocked(runtimeState);
const mockModels = vi.mocked(runtimeModels);
const mockStatus = vi.mocked(runtimeModelStatus);
const mockInstallServer = vi.mocked(runtimeInstallServer);
const mockInstallModel = vi.mocked(runtimeInstallModel);
const mockCancel = vi.mocked(runtimeCancelDownload);
const mockProgress = vi.mocked(onDownloadProgress);

let emitProgress: (p: DownloadProgress) => void;

const MODELS = [
  {
    id: "qwen2.5-3b",
    name: "Qwen2.5 3B Instruct",
    url: "https://example.test/a.gguf",
    filename: "a.gguf",
    approxBytes: 2_104_932_768,
    note: "Fast.",
  },
];

beforeEach(() => {
  for (const m of [
    mockState,
    mockModels,
    mockStatus,
    mockInstallServer,
    mockInstallModel,
    mockCancel,
    mockProgress,
  ]) {
    m.mockReset();
  }
  mockState.mockResolvedValue({ state: "not_installed" });
  mockModels.mockResolvedValue(MODELS);
  mockStatus.mockResolvedValue([]);
  mockInstallServer.mockResolvedValue(undefined);
  mockInstallModel.mockResolvedValue(undefined);
  mockCancel.mockResolvedValue(undefined);
  mockProgress.mockImplementation(async (handler) => {
    emitProgress = handler;
    return () => {};
  });
});

describe("formatBytes", () => {
  it("scales to a unit a person can read", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2_104_932_768)).toBe("2.0 GB");
    expect(formatBytes(4_683_074_240)).toBe("4.4 GB");
  });
});

describe("downloadLabel", () => {
  it("offers a resume rather than implying a fresh start", () => {
    // Getting this wrong makes the user think they are about to re-fetch
    // gigabytes they already have.
    expect(downloadLabel({ status: "partial", bytes: 1_073_741_824 })).toBe(
      "Resume from 1.0 GB",
    );
    expect(downloadLabel({ status: "absent" })).toBe("Download");
    expect(downloadLabel({ status: "installed" })).toBe("Installed");
    expect(downloadLabel(undefined)).toBe("Download");
  });
});

describe("ModelPicker", () => {
  it("asks for the server before offering models", async () => {
    // Downloading four gigabytes of model with nothing to run it is the worst
    // possible ordering.
    render(<ModelPicker />);

    expect(
      await screen.findByRole("button", { name: /download \(~11 MB\)/i }),
    ).toBeInTheDocument();
    expect(await screen.findByText(/install the server first/i)).toBeInTheDocument();
    expect(screen.queryByText(/Qwen2\.5 3B/)).toBeNull();
  });

  it("offers the models once the server is installed", async () => {
    mockState.mockResolvedValue({ state: "needs_model" });
    render(<ModelPicker />);

    expect(await screen.findByText(/Qwen2\.5 3B/)).toBeInTheDocument();
    expect(screen.getByText("2.0 GB")).toBeInTheDocument();
  });

  it("downloads the server and refreshes what is installed", async () => {
    render(<ModelPicker />);
    fireEvent.click(
      await screen.findByRole("button", { name: /download \(~11 MB\)/i }),
    );

    await waitFor(() => expect(mockInstallServer).toHaveBeenCalled());
    // Refreshed afterwards, or the card would still say nothing is installed.
    await waitFor(() => expect(mockState.mock.calls.length).toBeGreaterThan(1));
  });

  it("shows how far a download has got", async () => {
    mockState.mockResolvedValue({ state: "needs_model" });
    let release: () => void = () => {};
    mockInstallModel.mockImplementation(
      () => new Promise((resolve) => (release = () => resolve())),
    );

    render(<ModelPicker />);
    fireEvent.click(await screen.findByRole("button", { name: /^download$/i }));

    emitProgress({
      id: "qwen2.5-3b",
      downloaded: 500_000_000,
      total: 2_000_000_000,
      done: false,
    });

    expect(await screen.findByText(/477 MB of 1.9 GB/)).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("progress-fill")).toHaveStyle({ width: "25%" }),
    );
    release();
  });

  it("keeps the bar at zero rather than crashing when no total is known", async () => {
    // A server that sends no Content-Length would otherwise divide by null.
    mockState.mockResolvedValue({ state: "needs_model" });
    let release: () => void = () => {};
    mockInstallModel.mockImplementation(
      () => new Promise((resolve) => (release = () => resolve())),
    );

    render(<ModelPicker />);
    fireEvent.click(await screen.findByRole("button", { name: /^download$/i }));
    emitProgress({ id: "qwen2.5-3b", downloaded: 1000, total: null, done: false });

    expect(await screen.findByText(/^1000 B$/)).toBeInTheDocument();
    expect(screen.getByTestId("progress-fill")).toHaveStyle({ width: "0%" });
    release();
  });

  it("can cancel, and says the bytes are kept", async () => {
    mockState.mockResolvedValue({ state: "needs_model" });
    let release: () => void = () => {};
    mockInstallModel.mockImplementation(
      () => new Promise((resolve) => (release = () => resolve())),
    );

    render(<ModelPicker />);
    fireEvent.click(await screen.findByRole("button", { name: /^download$/i }));

    fireEvent.click(await screen.findByRole("button", { name: /cancel/i }));
    await waitFor(() => expect(mockCancel).toHaveBeenCalled());
    expect(screen.getByText(/keeps what has downloaded/i)).toBeInTheDocument();
    release();
  });

  it("does not report a cancellation as an error", async () => {
    // Cancelling is a choice, not a fault.
    mockState.mockResolvedValue({ state: "needs_model" });
    mockInstallModel.mockRejectedValue("cancelled");

    render(<ModelPicker />);
    fireEvent.click(await screen.findByRole("button", { name: /^download$/i }));

    await waitFor(() => expect(mockInstallModel).toHaveBeenCalled());
    expect(screen.queryByText(/cancelled/i)).toBeNull();
  });

  it("surfaces a real failure", async () => {
    mockState.mockResolvedValue({ state: "needs_model" });
    mockInstallModel.mockRejectedValue("the URL may have moved");

    render(<ModelPicker />);
    fireEvent.click(await screen.findByRole("button", { name: /^download$/i }));

    expect(await screen.findByText(/URL may have moved/)).toBeInTheDocument();
  });

  it("will not re-download a model that is already installed", async () => {
    mockState.mockResolvedValue({ state: "ready" });
    mockStatus.mockResolvedValue([["qwen2.5-3b", { status: "installed" }]]);

    render(<ModelPicker />);
    const button = await screen.findByRole("button", { name: /installed/i });
    expect(button).toBeDisabled();
  });

  it("offers to resume a partial download from where it stopped", async () => {
    mockState.mockResolvedValue({ state: "needs_model" });
    mockStatus.mockResolvedValue([
      ["qwen2.5-3b", { status: "partial", bytes: 1_073_741_824 }],
    ]);

    render(<ModelPicker />);
    expect(
      await screen.findByRole("button", { name: /resume from 1\.0 GB/i }),
    ).toBeInTheDocument();
  });
});
