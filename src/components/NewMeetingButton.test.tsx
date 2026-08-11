import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NewMeetingButton } from "./NewMeetingButton";
import { meetingStart } from "../lib/tauri";

vi.mock("../lib/tauri", () => ({ meetingStart: vi.fn() }));
const mockCreate = vi.mocked(meetingStart);

beforeEach(() => {
  mockCreate.mockReset();
  mockCreate.mockResolvedValue("m-new");
});

describe("NewMeetingButton", () => {
  it("creates a meeting and opens it", async () => {
    const onCreated = vi.fn();
    render(<NewMeetingButton onCreated={onCreated} />);
    fireEvent.click(screen.getByRole("button", { name: /New note/ }));
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith("m-new"));
  });

  it("starts recording, because that is what the button now means", async () => {
    // A reversal of G38, asked for after using it: `+` used to open a page
    // and nothing else, so recording was a second step done late — after the
    // first minute of the meeting. It now records as it creates.
    //
    // The cost is that this needs a working sidecar and microphone, and fails
    // loudly when there is none. That is the right failure: a note that
    // silently records nothing is the one worth avoiding.
    render(<NewMeetingButton onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /New note/ }));
    await waitFor(() => expect(mockCreate).toHaveBeenCalled());
    expect(mockCreate).toHaveBeenCalledWith();
  });

  it("reports a failure rather than doing nothing visible", async () => {
    mockCreate.mockRejectedValue("db lock poisoned");
    render(<NewMeetingButton onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /New note/ }));
    expect(await screen.findByText(/db lock poisoned/)).toBeInTheDocument();
  });

  it("cannot be pressed twice into two meetings", async () => {
    mockCreate.mockImplementation(() => new Promise(() => {}));
    render(<NewMeetingButton onCreated={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /New note/ }));
    expect(screen.getByRole("button", { name: /New note/ })).toBeDisabled();
  });
});
