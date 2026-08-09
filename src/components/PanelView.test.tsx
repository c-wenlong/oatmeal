import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PanelView } from "./PanelView";
import { panelDelete, panelGenerate, panelsList, templatesList } from "../lib/tauri";
import type { Panel, PanelContent } from "../types";

vi.mock("../lib/tauri", () => ({
  templatesList: vi.fn(),
  panelsList: vi.fn(),
  panelGenerate: vi.fn(),
  panelDelete: vi.fn(),
}));

const mockTemplates = vi.mocked(templatesList);
const mockList = vi.mocked(panelsList);
const mockGenerate = vi.mocked(panelGenerate);
const mockDelete = vi.mocked(panelDelete);

function panel(id: string, content: PanelContent, generatedAt = 1_000): Panel {
  return {
    id,
    templateId: "default",
    contentJson: JSON.stringify(content),
    provider: "Ollama",
    model: "llama3.2",
    generatedAt,
  };
}

const cited: PanelContent = {
  sections: [
    {
      heading: "Decisions",
      bullets: [
        {
          text: "Ship on Thursday",
          sourceUtterances: [12, 15],
          fromNote: null,
        },
      ],
    },
  ],
};

// Block bodies: reset helpers return the mock, and vitest treats a function
// returned from a hook as a teardown callback.
beforeEach(() => {
  mockTemplates.mockReset();
  mockList.mockReset();
  mockGenerate.mockReset();
  mockDelete.mockReset();

  mockTemplates.mockResolvedValue([
    { id: "default", name: "Summary", prompt: "", isBuiltin: true },
    { id: "standup", name: "Standup", prompt: "", isBuiltin: true },
  ]);
  mockList.mockResolvedValue([]);
  mockDelete.mockResolvedValue(undefined);
});

describe("PanelView", () => {
  it("does not generate anything on mount", async () => {
    render(<PanelView meetingId="m1" />);
    await waitFor(() => expect(mockList).toHaveBeenCalled());
    expect(mockGenerate).not.toHaveBeenCalled();
  });

  it("renders sections and bullets from a stored panel", async () => {
    mockList.mockResolvedValue([panel("p1", cited)]);
    render(<PanelView meetingId="m1" />);

    expect(await screen.findByText("Decisions")).toBeInTheDocument();
    expect(screen.getByText("Ship on Thursday")).toBeInTheDocument();
  });

  it("renders a chip per citation and reports which line was clicked", async () => {
    mockList.mockResolvedValue([panel("p1", cited)]);
    const onCitationClick = vi.fn();
    render(<PanelView meetingId="m1" onCitationClick={onCitationClick} />);

    await userEvent.click(await screen.findByRole("button", { name: "#12" }));
    expect(onCitationClick).toHaveBeenCalledWith(12);

    await userEvent.click(screen.getByRole("button", { name: "#15" }));
    expect(onCitationClick).toHaveBeenCalledWith(15);
  });

  it("marks an uncited bullet rather than letting it look sourced", async () => {
    // A claim the citation gate stripped is the model's paraphrase. Rendering
    // it identically to a cited one would present it as equally traceable.
    mockList.mockResolvedValue([
      panel("p1", {
        sections: [
          {
            heading: "Summary",
            bullets: [
              { text: "Plausible claim", sourceUtterances: [], fromNote: null },
            ],
          },
        ],
      }),
    ]);
    render(<PanelView meetingId="m1" />);

    expect(await screen.findByText("uncited")).toBeInTheDocument();
  });

  it("distinguishes a bullet that came from the user's notes", async () => {
    mockList.mockResolvedValue([
      panel("p1", {
        sections: [
          {
            heading: "Summary",
            bullets: [
              { text: "You flagged this", sourceUtterances: [12], fromNote: "b3" },
            ],
          },
        ],
      }),
    ]);
    render(<PanelView meetingId="m1" />);
    expect(await screen.findByText("note")).toBeInTheDocument();
  });

  it("keeps the previous panel when regenerating", async () => {
    // The decision-gate default: regenerating forks rather than overwrites, so
    // a version the user preferred survives a retry.
    mockList.mockResolvedValue([panel("p1", cited, 1_000)]);
    mockGenerate.mockResolvedValue(
      panel(
        "p2",
        {
          sections: [
            {
              heading: "Standup",
              bullets: [{ text: "New take", sourceUtterances: [], fromNote: null }],
            },
          ],
        },
        2_000,
      ),
    );

    render(<PanelView meetingId="m1" />);
    await userEvent.click(await screen.findByLabelText("template"));
    await userEvent.click(screen.getByLabelText(/^(re)?generate$/));

    expect(await screen.findByText("New take")).toBeInTheDocument();

    // Both versions remain selectable — the earlier one was not overwritten.
    const versions = document.querySelectorAll(".panel-version");
    expect(versions).toHaveLength(2);

    await userEvent.click(versions[1] as HTMLElement);
    expect(await screen.findByText("Ship on Thursday")).toBeInTheDocument();
  });

  it("passes the chosen template through to generation", async () => {
    mockGenerate.mockResolvedValue(panel("p1", cited));
    render(<PanelView meetingId="m1" />);
    await waitFor(() => expect(mockTemplates).toHaveBeenCalled());

    await userEvent.click(screen.getByLabelText("template"));
    await userEvent.click(screen.getByRole("menuitem", { name: "Standup" }));

    expect(mockGenerate).toHaveBeenCalledWith("m1", "standup");
  });

  it("reports which model produced the panel", async () => {
    // The privacy story rests on this being per-generation, not per-app.
    mockList.mockResolvedValue([panel("p1", cited)]);
    render(<PanelView meetingId="m1" />);

    // Provenance moved into the template pill's menu: still one click away,
    // no longer printed under every summary in the document.
    await userEvent.click(await screen.findByLabelText("template"));
    expect(await screen.findByText(/Ollama.*llama3\.2/)).toBeInTheDocument();
  });

  it("surfaces a generation failure instead of an empty summary", async () => {
    mockGenerate.mockRejectedValue(new Error("OpenAI needs an API key"));
    render(<PanelView meetingId="m1" />);

    await userEvent.click(await screen.findByLabelText("template"));
    await userEvent.click(screen.getByLabelText(/^(re)?generate$/));
    expect(await screen.findByText(/needs an API key/)).toBeInTheDocument();
  });

  it("cannot generate without a meeting", async () => {
    render(<PanelView meetingId={null} />);
    // The pill itself is closed when there is no meeting to summarise.
    expect(await screen.findByLabelText("template")).toBeDisabled();
  });

  it("deletes a single version without touching the rest", async () => {
    mockList.mockResolvedValue([panel("p1", cited)]);
    render(<PanelView meetingId="m1" />);

    await userEvent.click(await screen.findByLabelText("template"));
    await userEvent.click(screen.getByRole("button", { name: /delete this version/i }));
    expect(mockDelete).toHaveBeenCalledWith("p1");
  });

  it("renders a panel whose stored JSON is corrupt as empty rather than crashing", async () => {
    const broken = { ...panel("p1", cited), contentJson: "{not json" };
    mockList.mockResolvedValue([broken]);
    render(<PanelView meetingId="m1" />);

    // No throw, no white screen.
    await waitFor(() => expect(mockList).toHaveBeenCalled());
    expect(screen.queryByText("Decisions")).not.toBeInTheDocument();
  });

  it("shows every bullet in a multi-section panel", async () => {
    mockList.mockResolvedValue([
      panel("p1", {
        sections: [
          {
            heading: "Decisions",
            bullets: [{ text: "Ship Thursday", sourceUtterances: [1], fromNote: null }],
          },
          {
            heading: "Action items",
            bullets: [
              { text: "Own the rollback", sourceUtterances: [2], fromNote: null },
            ],
          },
        ],
      }),
    ]);
    render(<PanelView meetingId="m1" />);

    expect(await screen.findByText("Decisions")).toBeInTheDocument();
    const actions = screen.getByText("Action items").closest(".panel-section")!;
    expect(
      within(actions as HTMLElement).getByText("Own the rollback"),
    ).toBeInTheDocument();
  });
});
