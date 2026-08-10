import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SidecarLogCard, calendarLines, withoutStamp } from "./SidecarLogCard";
import { sidecarLogPath, sidecarLogTail } from "../lib/tauri";

vi.mock("../lib/tauri", () => ({
  sidecarLogTail: vi.fn(),
  sidecarLogPath: vi.fn(),
}));

const mockTail = vi.mocked(sidecarLogTail);
const mockPath = vi.mocked(sidecarLogPath);

const LINES = [
  "1770000000000 [supervisor] spawned pid=42 attempt=1",
  "1770000000100 [sidecar] ready 0.1.1",
  "1770000000200 [calendar] authorized=false calendars=0 events=0",
];

beforeEach(() => {
  mockTail.mockReset();
  mockPath.mockReset();
  mockTail.mockResolvedValue(LINES);
  mockPath.mockResolvedValue("/tmp/sidecar.log");
});

describe("calendarLines", () => {
  it("keeps the calendar lines and drops the rest", () => {
    // The whole log is mostly audio. Someone asking why a calendar list is
    // empty should not have to read a transcript to find out.
    expect(calendarLines(LINES)).toEqual([LINES[2]]);
  });
});

describe("withoutStamp", () => {
  it("hides the epoch millis but not the line", () => {
    expect(withoutStamp(LINES[1])).toBe("[sidecar] ready 0.1.1");
  });

  it("leaves a line that has no stamp alone", () => {
    expect(withoutStamp("[sidecar] ready")).toBe("[sidecar] ready");
  });
});

describe("SidecarLogCard", () => {
  it("shows what the sidecar reported", async () => {
    render(<SidecarLogCard />);
    expect(
      await screen.findByText(/\[calendar\] authorized=false/),
    ).toBeInTheDocument();
  });

  it("filters to the calendar when asked", async () => {
    render(<SidecarLogCard />);
    fireEvent.click(await screen.findByRole("button", { name: /calendar only/i }));
    expect(screen.queryByText(/\[sidecar\] ready/)).toBeNull();
    expect(screen.getByText(/\[calendar\] authorized=false/)).toBeInTheDocument();
  });

  it("says why it is empty rather than showing an empty box", async () => {
    mockTail.mockResolvedValue([]);
    render(<SidecarLogCard />);
    expect(await screen.findByText(/Nothing logged yet/i)).toBeInTheDocument();
  });

  it("distinguishes no log from no calendar lines in it", async () => {
    // Both render an empty list; only one of them means something is wrong.
    mockTail.mockResolvedValue(["1770000000000 [sidecar] ready 0.1.1"]);
    render(<SidecarLogCard />);
    fireEvent.click(await screen.findByRole("button", { name: /calendar only/i }));
    expect(screen.getByText(/No calendar lines/i)).toBeInTheDocument();
  });

  it("shows the path, so a log can be attached rather than retyped", async () => {
    render(<SidecarLogCard />);
    await waitFor(() =>
      expect(screen.getByText("/tmp/sidecar.log")).toBeInTheDocument(),
    );
  });
});
