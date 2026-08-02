import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Notepad } from "./Notepad";
import { notesLoad, notesSave } from "../lib/tauri";
import type { NoteBlock } from "../types";

vi.mock("../lib/tauri", () => ({
  notesLoad: vi.fn(),
  notesSave: vi.fn(),
}));

const mockLoad = vi.mocked(notesLoad);
const mockSave = vi.mocked(notesSave);

// Block bodies: the reset helpers return the mock, and vitest treats a function
// returned from a hook as a teardown callback.
beforeEach(() => {
  mockLoad.mockReset();
  mockSave.mockReset();
  mockLoad.mockResolvedValue([]);
  mockSave.mockResolvedValue(undefined);
});

/** Last payload handed to `notesSave`. */
function lastSaved(): NoteBlock[] {
  const calls = mockSave.mock.calls;
  return calls[calls.length - 1][1];
}

describe("Notepad", () => {
  it("loads whatever is already stored for the meeting", async () => {
    mockLoad.mockResolvedValue([
      {
        blockId: "b1",
        seq: 0,
        text: "deadline is the 14th",
        firstTypedAtMs: 5_000,
        lastEditedAtMs: 5_000,
      },
    ]);

    render(<Notepad meetingId="m1" elapsedMs={() => 0} />);

    expect(await screen.findByText("deadline is the 14th")).toBeInTheDocument();
    expect(mockLoad).toHaveBeenCalledWith("m1");
  });

  it("autosaves typed notes with the elapsed anchor", async () => {
    let clock = 0;
    render(<Notepad meetingId="m1" elapsedMs={() => clock} />);
    await waitFor(() => expect(mockLoad).toHaveBeenCalled());

    clock = 12_000;
    await userEvent.click(await screen.findByTestId("notepad"));
    await userEvent.keyboard("deadline");

    await waitFor(() => expect(mockSave).toHaveBeenCalled(), { timeout: 3000 });

    const blocks = lastSaved();
    expect(blocks).toHaveLength(1);
    expect(blocks[0].text).toBe("deadline");
    // The anchor is what the temporal linker keys on.
    expect(blocks[0].firstTypedAtMs).toBe(12_000);
  });

  it("gives each block a distinct id", async () => {
    render(<Notepad meetingId="m1" elapsedMs={() => 1_000} />);
    await waitFor(() => expect(mockLoad).toHaveBeenCalled());

    await userEvent.click(await screen.findByTestId("notepad"));
    await userEvent.keyboard("first{Enter}second");

    await waitFor(() => expect(mockSave).toHaveBeenCalled(), { timeout: 3000 });

    const blocks = lastSaved();
    expect(blocks).toHaveLength(2);
    // Position is not identity — two blocks must never share an id, or one
    // would inherit the other's anchor.
    expect(blocks[0].blockId).not.toBe(blocks[1].blockId);
    expect(blocks.map((b) => b.text)).toEqual(["first", "second"]);
  });

  it("keeps the original anchor when a block is edited later", async () => {
    let clock = 3_000;
    render(<Notepad meetingId="m1" elapsedMs={() => clock} />);
    await waitFor(() => expect(mockLoad).toHaveBeenCalled());

    await userEvent.click(await screen.findByTestId("notepad"));
    await userEvent.keyboard("deadline");
    await waitFor(() => expect(mockSave).toHaveBeenCalled(), { timeout: 3000 });

    clock = 120_000;
    await userEvent.keyboard(" is the 14th");
    await waitFor(() => expect(lastSaved()[0].text).toContain("14th"), {
      timeout: 3000,
    });

    expect(lastSaved()[0].firstTypedAtMs).toBe(3_000);
  });

  it("does not save the trailing empty paragraph", async () => {
    render(<Notepad meetingId="m1" elapsedMs={() => 0} />);
    await waitFor(() => expect(mockLoad).toHaveBeenCalled());

    await userEvent.click(await screen.findByTestId("notepad"));
    await userEvent.keyboard("a note{Enter}");

    await waitFor(() => expect(mockSave).toHaveBeenCalled(), { timeout: 3000 });
    expect(lastSaved()).toHaveLength(1);
  });

  it("saves nothing when there is no meeting to save against", async () => {
    render(<Notepad meetingId={null} elapsedMs={() => 0} />);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(mockLoad).not.toHaveBeenCalled();
    expect(mockSave).not.toHaveBeenCalled();
  });

  it("reports a failed save instead of appearing to have saved", async () => {
    mockSave.mockRejectedValue(new Error("disk full"));
    render(<Notepad meetingId="m1" elapsedMs={() => 0} />);
    await waitFor(() => expect(mockLoad).toHaveBeenCalled());

    await userEvent.click(await screen.findByTestId("notepad"));
    await userEvent.keyboard("note");

    // Silently swallowing this would leave the user believing notes are safe.
    expect(
      await screen.findByText("not saved", {}, { timeout: 3000 }),
    ).toBeInTheDocument();
  });

  it("shows a saved indicator once notes are written", async () => {
    render(<Notepad meetingId="m1" elapsedMs={() => 0} />);
    await waitFor(() => expect(mockLoad).toHaveBeenCalled());

    await userEvent.click(await screen.findByTestId("notepad"));
    await userEvent.keyboard("note");

    expect(await screen.findByText("saved", {}, { timeout: 3000 })).toBeInTheDocument();
  });

  it("switches notes when the open meeting changes", async () => {
    mockLoad.mockResolvedValue([]);
    const { rerender } = render(<Notepad meetingId="m1" elapsedMs={() => 0} />);
    await waitFor(() => expect(mockLoad).toHaveBeenCalledWith("m1"));

    mockLoad.mockResolvedValue([
      {
        blockId: "other",
        seq: 0,
        text: "a different meeting",
        firstTypedAtMs: 0,
        lastEditedAtMs: 0,
      },
    ]);
    rerender(<Notepad meetingId="m2" elapsedMs={() => 0} />);

    expect(await screen.findByText("a different meeting")).toBeInTheDocument();
  });
});
