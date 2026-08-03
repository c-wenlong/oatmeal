import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ChatCard } from "./ChatCard";
import { chatAsk, foldersList } from "../lib/tauri";
import type { ChatReply } from "../types";

vi.mock("../lib/tauri", () => ({
  chatAsk: vi.fn(),
  foldersList: vi.fn(),
}));

const mockAsk = vi.mocked(chatAsk);
const mockFolders = vi.mocked(foldersList);

function reply(over: Partial<ChatReply> = {}): ChatReply {
  return {
    answer: {
      claims: [
        {
          text: "We committed to shipping on Thursday.",
          citations: [
            {
              utteranceId: 7,
              meetingId: "m1",
              meetingTitle: "Standup",
              startMs: 65_000,
            },
          ],
        },
      ],
    },
    report: { droppedCitations: 0, uncitedClaims: 0 },
    context: [
      {
        utteranceId: 7,
        meetingId: "m1",
        meetingTitle: "Standup",
        startMs: 65_000,
        text: "we ship on thursday",
      },
    ],
    ...over,
  };
}

beforeEach(() => {
  mockAsk.mockReset();
  mockFolders.mockReset();
  mockFolders.mockResolvedValue([]);
  mockAsk.mockResolvedValue(reply());
});

describe("ChatCard", () => {
  it("asks over everything by default", async () => {
    render(<ChatCard />);
    fireEvent.change(await screen.findByLabelText(/ask a question/i), {
      target: { value: "what did we commit to?" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    await waitFor(() =>
      expect(mockAsk).toHaveBeenCalledWith("what did we commit to?", null, null),
    );
  });

  it("scopes to the open meeting when there is one", async () => {
    // "What did we decide" almost always means the conversation on screen.
    render(<ChatCard meetingId="m1" />);
    fireEvent.change(await screen.findByLabelText(/ask a question/i), {
      target: { value: "what did we decide?" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    await waitFor(() =>
      expect(mockAsk).toHaveBeenCalledWith("what did we decide?", "m1", null),
    );
  });

  it("scopes to a folder when one is chosen", async () => {
    mockFolders.mockResolvedValue([
      { id: "f1", name: "Clients", parentId: null, meetingCount: 5 },
    ]);
    render(<ChatCard />);

    fireEvent.click(await screen.findByRole("button", { name: /Clients/ }));
    fireEvent.change(screen.getByLabelText(/ask a question/i), {
      target: { value: "commitments?" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    await waitFor(() =>
      expect(mockAsk).toHaveBeenCalledWith("commitments?", null, "f1"),
    );
  });

  it("shows a citation that points at a real moment", async () => {
    // G25's done-when: every claim carries a citation that resolves.
    render(<ChatCard />);
    fireEvent.change(await screen.findByLabelText(/ask a question/i), {
      target: { value: "q" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    const citation = await screen.findByRole("button", { name: /Standup 01:05/ });
    expect(citation).toBeInTheDocument();
  });

  it("reveals the cited moment when the chip is clicked", async () => {
    const onReveal = vi.fn();
    render(<ChatCard onReveal={onReveal} />);
    fireEvent.change(await screen.findByLabelText(/ask a question/i), {
      target: { value: "q" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    fireEvent.click(await screen.findByRole("button", { name: /Standup 01:05/ }));
    expect(onReveal).toHaveBeenCalledWith("m1", 7);
  });

  it("marks a claim it could not trace", async () => {
    // The model's inference, not something in the transcript. Letting it look
    // equally sourced is the failure this prevents.
    mockAsk.mockResolvedValue(
      reply({
        answer: { claims: [{ text: "Probably fine.", citations: [] }] },
        report: { droppedCitations: 0, uncitedClaims: 1 },
      }),
    );
    render(<ChatCard />);
    fireEvent.change(await screen.findByLabelText(/ask a question/i), {
      target: { value: "q" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    expect(await screen.findByText("uncited")).toBeInTheDocument();
  });

  it("says when citations were invented and dropped", async () => {
    // Surfaced rather than swallowed — a model inventing citations is worth
    // knowing about, and hiding it makes the gate invisible.
    mockAsk.mockResolvedValue(
      reply({ report: { droppedCitations: 2, uncitedClaims: 0 } }),
    );
    render(<ChatCard />);
    fireEvent.change(await screen.findByLabelText(/ask a question/i), {
      target: { value: "q" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    expect(
      await screen.findByText(/Dropped 2 citations that pointed at nothing/i),
    ).toBeInTheDocument();
  });

  it("will not ask an empty question", async () => {
    render(<ChatCard />);
    expect(await screen.findByRole("button", { name: /^ask$/i })).toBeDisabled();
  });

  it("surfaces a failure instead of showing a blank answer", async () => {
    mockAsk.mockRejectedValue("no provider configured");
    render(<ChatCard />);
    fireEvent.change(await screen.findByLabelText(/ask a question/i), {
      target: { value: "q" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    expect(await screen.findByText(/no provider configured/)).toBeInTheDocument();
  });

  it("says how much evidence the answer rested on", async () => {
    render(<ChatCard />);
    fireEvent.change(await screen.findByLabelText(/ask a question/i), {
      target: { value: "q" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^ask$/i }));

    expect(await screen.findByText(/1 transcript line\./)).toBeInTheDocument();
  });
});
