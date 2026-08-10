import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NewMeetingButton } from "./NewMeetingButton";
import { meetingCreate } from "../lib/tauri";

vi.mock("../lib/tauri", () => ({ meetingCreate: vi.fn() }));
const mockCreate = vi.mocked(meetingCreate);

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

  it("does not start a recording", async () => {
    // The whole point of the goal: `+` opens a page, it does not begin a
    // capture. If this ever routes through meetingStart it needs a microphone
    // and it lies about what it did.
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
