import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NotionCard, exportSummary } from "./NotionCard";
import {
  notionDatabases,
  notionExport,
  notionSetOptions,
  notionSetToken,
  notionSettings,
} from "../lib/tauri";
import type { NotionSettings } from "../types";

vi.mock("../lib/tauri", () => ({
  notionSettings: vi.fn(),
  notionSetToken: vi.fn(),
  notionSetOptions: vi.fn(),
  notionDatabases: vi.fn(),
  notionExport: vi.fn(),
}));

const mockGet = vi.mocked(notionSettings);
const mockToken = vi.mocked(notionSetToken);
const mockOptions = vi.mocked(notionSetOptions);
const mockDatabases = vi.mocked(notionDatabases);
const mockExport = vi.mocked(notionExport);

function settings(over: Partial<NotionSettings> = {}): NotionSettings {
  return {
    hasToken: true,
    databaseId: "db-1",
    includeTranscript: false,
    autoExport: false,
    ...over,
  };
}

beforeEach(() => {
  for (const m of [mockGet, mockToken, mockOptions, mockDatabases, mockExport]) {
    m.mockReset();
  }
  mockGet.mockResolvedValue(settings());
  mockToken.mockResolvedValue(undefined);
  mockOptions.mockResolvedValue(undefined);
  mockDatabases.mockResolvedValue([
    { id: "db-1", title: "Meetings", titleProperty: "Name", properties: ["Name"] },
  ]);
  mockExport.mockResolvedValue({ pageId: "page-1", created: true, blocks: 12 });
});

describe("exportSummary", () => {
  it("distinguishes a new page from an updated one", () => {
    // The whole point of the feature; the message has to say which happened.
    expect(exportSummary(true, 12)).toBe("Created a page with 12 blocks.");
    expect(exportSummary(false, 12)).toBe("Updated the existing page with 12 blocks.");
    expect(exportSummary(true, 1)).toBe("Created a page with 1 block.");
  });
});

describe("NotionCard", () => {
  it("hides everything until a token is stored", async () => {
    mockGet.mockResolvedValue(settings({ hasToken: false }));
    render(<NotionCard meetingId="m1" />);

    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    expect(screen.queryByLabelText(/notion database/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /export this meeting/i })).toBeNull();
  });

  it("never shows the stored token back", async () => {
    // The field is a password input and the placeholder only says one exists.
    mockGet.mockResolvedValue(settings());
    render(<NotionCard meetingId="m1" />);

    const field = await screen.findByLabelText(/notion integration token/i);
    expect(field).toHaveValue("");
    expect(field).toHaveAttribute("type", "password");
  });

  it("stores a pasted token", async () => {
    mockGet.mockResolvedValue(settings({ hasToken: false }));
    render(<NotionCard meetingId="m1" />);

    fireEvent.change(await screen.findByLabelText(/notion integration token/i), {
      target: { value: "ntn_secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save token/i }));

    await waitFor(() => expect(mockToken).toHaveBeenCalledWith("ntn_secret"));
  });

  it("clears the token with an empty save", async () => {
    render(<NotionCard meetingId="m1" />);
    fireEvent.click(await screen.findByRole("button", { name: /remove token/i }));
    await waitFor(() => expect(mockToken).toHaveBeenCalledWith(""));
  });

  it("explains an empty database list rather than showing a blank dropdown", async () => {
    // Notion only returns what has been shared; empty is a normal state.
    mockDatabases.mockResolvedValue([]);
    render(<NotionCard meetingId="m1" />);

    fireEvent.click(await screen.findByRole("button", { name: /find databases/i }));
    expect(
      await screen.findByText(/Connections → add your integration/),
    ).toBeInTheDocument();
  });

  it("remembers the chosen database", async () => {
    render(<NotionCard meetingId="m1" />);
    fireEvent.click(await screen.findByRole("button", { name: /find databases/i }));

    await screen.findByRole("option", { name: "Meetings" });
    fireEvent.change(screen.getByLabelText(/notion database/i), {
      target: { value: "db-1" },
    });

    await waitFor(() => expect(mockOptions).toHaveBeenCalledWith("db-1", false, false));
  });

  it("can include the transcript", async () => {
    render(<NotionCard meetingId="m1" />);
    fireEvent.click(await screen.findByLabelText(/include the full transcript/i));
    await waitFor(() => expect(mockOptions).toHaveBeenCalledWith("db-1", true, false));
  });

  it("can export automatically when a meeting ends", async () => {
    render(<NotionCard meetingId="m1" />);
    fireEvent.click(await screen.findByLabelText(/export automatically/i));
    await waitFor(() => expect(mockOptions).toHaveBeenCalledWith("db-1", false, true));
  });

  it("exports the open meeting", async () => {
    render(<NotionCard meetingId="m1" />);
    fireEvent.click(
      await screen.findByRole("button", { name: /export this meeting/i }),
    );

    await waitFor(() => expect(mockExport).toHaveBeenCalledWith("m1"));
    expect(
      await screen.findByText(/Created a page with 12 blocks/),
    ).toBeInTheDocument();
  });

  it("says when it updated rather than created", async () => {
    // The done-when made visible: a second export must not look like a first.
    mockExport.mockResolvedValue({ pageId: "page-1", created: false, blocks: 9 });
    render(<NotionCard meetingId="m1" />);

    fireEvent.click(
      await screen.findByRole("button", { name: /export this meeting/i }),
    );
    expect(
      await screen.findByText(/Updated the existing page with 9 blocks/),
    ).toBeInTheDocument();
  });

  it("will not export without a meeting open", async () => {
    render(<NotionCard meetingId={null} />);
    expect(
      await screen.findByRole("button", { name: /export this meeting/i }),
    ).toBeDisabled();
    expect(screen.getByText(/Open a meeting first/)).toBeInTheDocument();
  });

  it("will not export without a database chosen", async () => {
    mockGet.mockResolvedValue(settings({ databaseId: null }));
    render(<NotionCard meetingId="m1" />);

    expect(
      await screen.findByRole("button", { name: /export this meeting/i }),
    ).toBeDisabled();
    expect(screen.getByText(/Choose a database first/)).toBeInTheDocument();
  });

  it("surfaces an export failure", async () => {
    mockExport.mockRejectedValue("the chosen Notion database is no longer shared");
    render(<NotionCard meetingId="m1" />);

    fireEvent.click(
      await screen.findByRole("button", { name: /export this meeting/i }),
    );
    expect(await screen.findByText(/no longer shared/)).toBeInTheDocument();
  });
});
