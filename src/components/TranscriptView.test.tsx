import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TranscriptView, offsetLabel, speakerLabel } from "./TranscriptView";
import type { Utterance } from "../types";

const line = (over: Partial<Utterance> = {}): Utterance => ({
  id: 1,
  seq: 1,
  source: "mic",
  text: "Are you transcribing?",
  startMs: 0,
  endMs: 1000,
  confidence: null,
  ...over,
});

describe("offsetLabel", () => {
  it("counts from the start of the meeting, not the clock", () => {
    // "8:47 PM" says when the meeting was; "12:05" says where in it you are.
    expect(offsetLabel(0)).toBe("0:00");
    expect(offsetLabel(65_000)).toBe("1:05");
    expect(offsetLabel(3_725_000)).toBe("62:05");
  });

  it("does not render a negative offset", () => {
    expect(offsetLabel(-500)).toBe("0:00");
  });
});

describe("speakerLabel", () => {
  it("names the people, not the plumbing", () => {
    // "Mic" and "System" describe where the audio came from. The reader wants
    // to know who was talking.
    expect(speakerLabel("mic")).toBe("You");
    expect(speakerLabel("system")).toBe("Them");
  });
});

describe("TranscriptView", () => {
  it("shows every line", () => {
    render(
      <TranscriptView
        utterances={[line({ id: 1 }), line({ id: 2, text: "Yes", source: "system" })]}
      />,
    );
    expect(screen.getByText("Are you transcribing?")).toBeInTheDocument();
    expect(screen.getByText("Yes")).toBeInTheDocument();
    expect(screen.getByText("Them")).toBeInTheDocument();
  });

  it("explains an empty transcript rather than showing a blank pane", () => {
    // The most likely cause is worth naming: the model was still loading when
    // the recording started, which is how a real recording produced nothing.
    render(<TranscriptView utterances={[]} />);
    expect(screen.getByTestId("transcript-empty").textContent).toMatch(
      /still have been loading/,
    );
  });

  it("reports which line was clicked", () => {
    const onSeek = vi.fn();
    render(<TranscriptView utterances={[line({ id: 42 })]} onSeek={onSeek} />);
    fireEvent.click(screen.getByRole("button", { name: "0:00" }));
    expect(onSeek).toHaveBeenCalledWith(42);
  });

  it("reads without a seek handler", () => {
    // The list is worth showing even where nothing can be revealed from it.
    render(<TranscriptView utterances={[line()]} />);
    fireEvent.click(screen.getByRole("button", { name: "0:00" }));
  });
});
