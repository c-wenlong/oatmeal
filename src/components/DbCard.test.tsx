import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DbCard } from "./DbCard";
import { dbSelftest } from "../lib/tauri";
import type { DbSelftest } from "../types";

vi.mock("../lib/tauri", () => ({ dbSelftest: vi.fn() }));

const mockSelftest = vi.mocked(dbSelftest);

const passing: DbSelftest = {
  schemaVersion: 2,
  dbPath: "/Users/test/oatmeal.sqlite",
  stats: {
    schemaVersion: 2,
    meetings: 3,
    utterances: 120,
    noteBlocks: 8,
    panels: 4,
    embeddings: 128,
  },
  ftsHit: "So the deadline for the migration is the fourteenth.",
  vectorHit: "near",
};

describe("DbCard", () => {
  // Block body, not `() => mockSelftest.mockReset()`. The reset helpers return
  // the mock for chaining, and vitest treats a function returned from a hook as
  // a teardown callback — so a concise arrow makes vitest *call the mock* after
  // every test, which throws whenever a test installed a throwing implementation.
  beforeEach(() => {
    mockSelftest.mockReset();
  });

  it("does not run the self-test until asked", () => {
    render(<DbCard />);
    expect(screen.getByText("not run")).toBeInTheDocument();
    expect(mockSelftest).not.toHaveBeenCalled();
  });

  it("reports passing when stemming and vector search both work", async () => {
    mockSelftest.mockResolvedValue(passing);
    render(<DbCard />);

    await userEvent.click(screen.getByRole("button", { name: /run self-test/i }));

    expect(await screen.findByText("passing")).toBeInTheDocument();
    expect(screen.getByText(/"migrate" matched/)).toBeInTheDocument();
    expect(screen.getByText(/nearest = "near"/)).toBeInTheDocument();
    expect(screen.getByText(/3 meetings/)).toBeInTheDocument();
  });

  it("reports degraded rather than passing when stemming silently fails", async () => {
    mockSelftest.mockResolvedValue({ ...passing, ftsHit: null });
    render(<DbCard />);

    await userEvent.click(screen.getByRole("button", { name: /run self-test/i }));

    expect(await screen.findByText("degraded")).toBeInTheDocument();
    expect(screen.getByText(/no hit for stemmed query/)).toBeInTheDocument();
  });

  it("reports degraded when the vector index returns the wrong neighbour", async () => {
    mockSelftest.mockResolvedValue({ ...passing, vectorHit: "far" });
    render(<DbCard />);

    await userEvent.click(screen.getByRole("button", { name: /run self-test/i }));

    expect(await screen.findByText("degraded")).toBeInTheDocument();
  });

  it("surfaces a thrown error", async () => {
    mockSelftest.mockRejectedValue(new Error("no such table: embeddings"));
    render(<DbCard />);

    await userEvent.click(screen.getByRole("button", { name: /run self-test/i }));

    expect(await screen.findByText("failed")).toBeInTheDocument();
    expect(screen.getByText(/no such table/)).toBeInTheDocument();
  });
});
