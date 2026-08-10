import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AskBar, surfaceTitle } from "./AskBar";

// The bar composes three real components. This test is about where they live
// and when they appear, not about what any of them does.
vi.mock("./RecordControl", () => ({
  RecordControl: () => <div data-testid="record-control" />,
}));
vi.mock("./ChatCard", () => ({
  ChatCard: ({ meetingId }: { meetingId: string | null }) => (
    <div data-testid="chat">{meetingId ?? "corpus"}</div>
  ),
}));
vi.mock("./SearchCard", () => ({
  SearchCard: () => <div data-testid="search" />,
}));

describe("surfaceTitle", () => {
  it("names each surface and nothing when closed", () => {
    expect(surfaceTitle("ask")).toBe("Ask");
    expect(surfaceTitle("search")).toBe("Search");
    expect(surfaceTitle(null)).toBeNull();
  });
});

describe("AskBar", () => {
  it("shows recording and asking together", () => {
    // The two things always available, in one place, per the teardown.
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    expect(screen.getByTestId("record-control")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Ask anything" })).toBeInTheDocument();
  });

  it("opens nothing until asked", () => {
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opens Ask", () => {
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Ask anything" }));
    expect(screen.getByTestId("chat")).toBeInTheDocument();
  });

  it("opens Search", () => {
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    fireEvent.click(screen.getByLabelText("search"));
    expect(screen.getByTestId("search")).toBeInTheDocument();
  });

  it("scopes Ask to the meeting in view", () => {
    // On a document the question is about this meeting; on the library it is
    // about the whole corpus. Passing the wrong one silently answers from the
    // wrong evidence.
    render(<AskBar meetingId="m1" onReveal={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Ask anything" }));
    expect(screen.getByTestId("chat")).toHaveTextContent("m1");
  });

  it("asks about the whole corpus when no meeting is open", () => {
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Ask anything" }));
    expect(screen.getByTestId("chat")).toHaveTextContent("corpus");
  });

  it("switches between the two without closing", () => {
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Ask anything" }));
    fireEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(screen.getByTestId("search")).toBeInTheDocument();
    expect(screen.queryByTestId("chat")).toBeNull();
  });

  it("closes on the close button", () => {
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Ask anything" }));
    fireEvent.click(screen.getByLabelText("close"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes on Escape", () => {
    // A sheet over the document that only closes by aiming at a small ✕ is a
    // sheet people leave open.
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Ask anything" }));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps recording reachable while the sheet is open", () => {
    // Losing the stop button behind a dialog would mean a recording you cannot
    // end without dismissing something first.
    render(<AskBar meetingId={null} onReveal={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Ask anything" }));
    expect(screen.getByTestId("record-control")).toBeInTheDocument();
  });
});
