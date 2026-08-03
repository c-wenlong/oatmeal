import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SearchCard } from "./SearchCard";
import {
  folderCreate,
  folderDelete,
  folderMeetings,
  foldersList,
  meetingSetFolder,
  searchTranscripts,
} from "../lib/tauri";
import type { SearchResponse } from "../types";

vi.mock("../lib/tauri", () => ({
  foldersList: vi.fn(),
  folderCreate: vi.fn(),
  folderDelete: vi.fn(),
  folderMeetings: vi.fn(),
  meetingSetFolder: vi.fn(),
  searchTranscripts: vi.fn(),
}));

const mockFolders = vi.mocked(foldersList);
const mockCreate = vi.mocked(folderCreate);
const mockDelete = vi.mocked(folderDelete);
const mockInFolder = vi.mocked(folderMeetings);
const mockSetFolder = vi.mocked(meetingSetFolder);
const mockSearch = vi.mocked(searchTranscripts);

function response(over: Partial<SearchResponse> = {}): SearchResponse {
  return {
    semantic: true,
    results: [
      {
        meetingId: "m1",
        title: "Vendor review",
        startedAt: 0,
        bestAtMs: 42_000,
        bestUtteranceId: 9,
        score: 0.5,
        hits: [
          {
            utteranceId: 9,
            meetingId: "m1",
            text: "the rollback plan is mine",
            startMs: 42_000,
            kind: "both",
            score: 0.5,
          },
        ],
        previews: [
          {
            text: "the rollback plan is mine",
            spans: [[4, 12]],
            truncatedStart: false,
            truncatedEnd: false,
          },
        ],
      },
    ],
    ...over,
  };
}

beforeEach(() => {
  for (const m of [
    mockFolders,
    mockCreate,
    mockDelete,
    mockInFolder,
    mockSetFolder,
    mockSearch,
  ]) {
    m.mockReset();
  }
  mockFolders.mockResolvedValue([]);
  mockCreate.mockResolvedValue("f-new");
  mockDelete.mockResolvedValue(undefined);
  mockInFolder.mockResolvedValue([]);
  mockSetFolder.mockResolvedValue(undefined);
  mockSearch.mockResolvedValue(response());
});

describe("SearchCard", () => {
  it("does not search until something is typed", async () => {
    render(<SearchCard />);
    await waitFor(() => expect(mockFolders).toHaveBeenCalled());
    expect(mockSearch).not.toHaveBeenCalled();
  });

  it("groups results under their meeting", async () => {
    render(<SearchCard />);
    fireEvent.change(screen.getByLabelText(/search transcripts/i), {
      target: { value: "rollback" },
    });

    expect(await screen.findByText("Vendor review")).toBeInTheDocument();
    expect(screen.getByText("1 match")).toBeInTheDocument();
  });

  it("highlights the matched words", async () => {
    render(<SearchCard />);
    fireEvent.change(screen.getByLabelText(/search transcripts/i), {
      target: { value: "rollback" },
    });

    const marked = await screen.findByText("rollback");
    expect(marked.tagName).toBe("MARK");
  });

  it("jumps to the matched moment", async () => {
    // G24's done-when: the right meeting *and* the right moment.
    const onReveal = vi.fn();
    render(<SearchCard onReveal={onReveal} />);
    fireEvent.change(screen.getByLabelText(/search transcripts/i), {
      target: { value: "rollback" },
    });

    fireEvent.click(await screen.findByRole("button", { name: "00:42" }));
    expect(onReveal).toHaveBeenCalledWith("m1", 9);
  });

  it("says when the search was keyword-only", async () => {
    // Otherwise a user with no embedder silently gets worse results and no
    // reason why.
    mockSearch.mockResolvedValue(response({ semantic: false }));
    render(<SearchCard />);
    fireEvent.change(screen.getByLabelText(/search transcripts/i), {
      target: { value: "rollback" },
    });

    expect(
      await screen.findByText(/no embedding model is reachable/i),
    ).toBeInTheDocument();
  });

  it("says plainly when nothing matched", async () => {
    mockSearch.mockResolvedValue(response({ results: [] }));
    render(<SearchCard />);
    fireEvent.change(screen.getByLabelText(/search transcripts/i), {
      target: { value: "zzz" },
    });

    expect(await screen.findByText(/Nothing matched/)).toBeInTheDocument();
  });

  it("scopes the search to a chosen folder", async () => {
    mockFolders.mockResolvedValue([
      { id: "f1", name: "Clients", parentId: null, meetingCount: 3 },
    ]);
    render(<SearchCard />);

    fireEvent.click(await screen.findByRole("button", { name: /Clients \(3\)/ }));
    fireEvent.change(screen.getByLabelText(/search transcripts/i), {
      target: { value: "rollback" },
    });

    await waitFor(() => expect(mockSearch).toHaveBeenCalledWith("rollback", "f1"));
  });

  it("warns that deleting a folder keeps its meetings", async () => {
    // "Delete folder" reads like it might take the recordings with it. It does
    // not, and the confirmation has to say so.
    mockFolders.mockResolvedValue([
      { id: "f1", name: "Clients", parentId: null, meetingCount: 3 },
    ]);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<SearchCard />);

    fireEvent.click(
      await screen.findByRole("button", { name: /delete folder Clients/i }),
    );

    expect(confirm.mock.calls[0][0]).toMatch(/kept and become unfiled/i);
    await waitFor(() => expect(mockDelete).toHaveBeenCalledWith("f1"));
    confirm.mockRestore();
  });

  it("does not delete a folder when the confirmation is refused", async () => {
    mockFolders.mockResolvedValue([
      { id: "f1", name: "Clients", parentId: null, meetingCount: 3 },
    ]);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<SearchCard />);

    fireEvent.click(
      await screen.findByRole("button", { name: /delete folder Clients/i }),
    );
    expect(mockDelete).not.toHaveBeenCalled();
    confirm.mockRestore();
  });

  it("files a meeting into a folder", async () => {
    mockFolders.mockResolvedValue([
      { id: "f1", name: "Clients", parentId: null, meetingCount: 0 },
    ]);
    mockInFolder.mockResolvedValue([
      {
        id: "m1",
        title: "Vendor review",
        startedAt: 0,
        endedAt: null,
        status: "complete",
        audioPath: null,
        utteranceCount: 3,
      },
    ]);
    render(<SearchCard />);

    const list = await screen.findByTestId("filed-meetings");
    fireEvent.change(within(list).getByLabelText(/folder for Vendor review/i), {
      target: { value: "f1" },
    });

    await waitFor(() => expect(mockSetFolder).toHaveBeenCalledWith("m1", "f1"));
  });

  it("creates a folder from a prompt", async () => {
    const prompt = vi.spyOn(window, "prompt").mockReturnValue("Clients");
    render(<SearchCard />);

    fireEvent.click(await screen.findByRole("button", { name: /new folder/i }));
    await waitFor(() => expect(mockCreate).toHaveBeenCalledWith("Clients"));
    prompt.mockRestore();
  });

  it("does not create a folder with a blank name", async () => {
    const prompt = vi.spyOn(window, "prompt").mockReturnValue("   ");
    render(<SearchCard />);

    fireEvent.click(await screen.findByRole("button", { name: /new folder/i }));
    expect(mockCreate).not.toHaveBeenCalled();
    prompt.mockRestore();
  });
});
