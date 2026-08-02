import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import { HealthCard } from "./HealthCard";
import { healthCheck } from "../lib/tauri";

vi.mock("../lib/tauri", () => ({
  healthCheck: vi.fn(),
}));

const mockHealthCheck = vi.mocked(healthCheck);

describe("HealthCard", () => {
  beforeEach(() => {
    mockHealthCheck.mockReset();
  });

  it("shows the checking state before the core responds", () => {
    mockHealthCheck.mockReturnValue(new Promise(() => {}));
    render(<HealthCard />);
    expect(screen.getByText("checking")).toBeInTheDocument();
  });

  it("renders core details once connected", async () => {
    mockHealthCheck.mockResolvedValue({
      appVersion: "0.1.0",
      buildProfile: "debug",
      arch: "aarch64",
      os: "macos",
    });

    render(<HealthCard />);

    expect(await screen.findByText("connected")).toBeInTheDocument();
    expect(screen.getByText("0.1.0")).toBeInTheDocument();
    expect(screen.getByText("debug")).toBeInTheDocument();
    expect(screen.getByText("macos/aarch64")).toBeInTheDocument();
  });

  it("surfaces the failure instead of silently showing an empty card", async () => {
    mockHealthCheck.mockRejectedValue(new Error("ipc closed"));

    render(<HealthCard />);

    expect(await screen.findByText("unreachable")).toBeInTheDocument();
    expect(screen.getByText(/ipc closed/)).toBeInTheDocument();
  });
});
