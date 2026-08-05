import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  GoogleCalendarCard,
  connectMessage,
  looksLikeClientId,
} from "./GoogleCalendarCard";
import {
  gcalConnect,
  gcalDisconnect,
  gcalSetClientId,
  gcalSetEnabled,
  gcalSettings,
} from "../lib/tauri";
import type { GcalSettings } from "../types";

vi.mock("../lib/tauri", () => ({
  gcalSettings: vi.fn(),
  gcalSetClientId: vi.fn(),
  gcalSetEnabled: vi.fn(),
  gcalConnect: vi.fn(),
  gcalDisconnect: vi.fn(),
}));

const mockGet = vi.mocked(gcalSettings);
const mockSetId = vi.mocked(gcalSetClientId);
const mockSetEnabled = vi.mocked(gcalSetEnabled);
const mockConnect = vi.mocked(gcalConnect);
const mockDisconnect = vi.mocked(gcalDisconnect);

const CLIENT_ID = "123-abc.apps.googleusercontent.com";

function settings(over: Partial<GcalSettings> = {}): GcalSettings {
  return { connected: false, clientId: CLIENT_ID, enabled: false, ...over };
}

beforeEach(() => {
  for (const m of [mockGet, mockSetId, mockSetEnabled, mockConnect, mockDisconnect]) {
    m.mockReset();
  }
  mockGet.mockResolvedValue(settings());
  mockSetId.mockResolvedValue(undefined);
  mockSetEnabled.mockResolvedValue(undefined);
  mockConnect.mockResolvedValue({ connected: true, reason: null });
  mockDisconnect.mockResolvedValue(undefined);
});

describe("looksLikeClientId", () => {
  it("accepts a real client id", () => {
    expect(looksLikeClientId(CLIENT_ID)).toBe(true);
    expect(looksLikeClientId(`  ${CLIENT_ID}  `)).toBe(true);
  });

  it("rejects a client secret", () => {
    // The mistake people actually make. Pasting the secret fails much later
    // with an opaque Google error.
    expect(looksLikeClientId("GOCSPX-abc123def456")).toBe(false);
  });

  it("rejects junk", () => {
    expect(looksLikeClientId("")).toBe(false);
    expect(looksLikeClientId("123-abc")).toBe(false);
  });
});

describe("connectMessage", () => {
  it("confirms a connection", () => {
    expect(connectMessage(true, null)).toMatch(/Connected/);
  });

  it("explains a refusal in plain words", () => {
    expect(connectMessage(false, "access_denied")).toMatch(/You declined access/);
  });

  it("explains a state mismatch without jargon", () => {
    // This is the security check firing. The user needs to know nothing was
    // connected, not to read about OAuth.
    expect(connectMessage(false, "mismatched state — the request did not")).toMatch(
      /did not start/,
    );
  });

  it("passes an unrecognised reason through rather than swallowing it", () => {
    expect(connectMessage(false, "invalid_client")).toBe("invalid_client");
  });
});

describe("GoogleCalendarCard", () => {
  it("says it is only needed when macOS Calendar cannot help", async () => {
    // Otherwise people connect an account they did not need to.
    render(<GoogleCalendarCard />);
    // Matched on a contiguous run: the sentence contains an <em>, which splits
    // the text node and defeats a regex spanning it.
    expect(
      await screen.findByText(/Oatmeal already reads that one/i),
    ).toBeInTheDocument();
  });

  it("says no secret is needed", async () => {
    render(<GoogleCalendarCard />);
    expect(await screen.findByText(/no secret to copy/i)).toBeInTheDocument();
  });

  it("warns when a secret was pasted instead of an id", async () => {
    render(<GoogleCalendarCard />);
    fireEvent.change(await screen.findByLabelText(/google oauth client id/i), {
      target: { value: "GOCSPX-secret" },
    });
    expect(screen.getByText(/does not look like a client/i)).toBeInTheDocument();
  });

  it("saves a client id", async () => {
    mockGet.mockResolvedValue(settings({ clientId: null }));
    render(<GoogleCalendarCard />);

    fireEvent.change(await screen.findByLabelText(/google oauth client id/i), {
      target: { value: CLIENT_ID },
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() => expect(mockSetId).toHaveBeenCalledWith(CLIENT_ID));
  });

  it("will not connect without a client id", async () => {
    mockGet.mockResolvedValue(settings({ clientId: null }));
    render(<GoogleCalendarCard />);

    expect(
      await screen.findByRole("button", { name: /connect google calendar/i }),
    ).toBeDisabled();
    expect(screen.getByText(/Save a client id first/)).toBeInTheDocument();
  });

  it("runs the flow and reports success", async () => {
    render(<GoogleCalendarCard />);
    fireEvent.click(
      await screen.findByRole("button", { name: /connect google calendar/i }),
    );

    await waitFor(() => expect(mockConnect).toHaveBeenCalled());
    expect(await screen.findByText(/Connected\./)).toBeInTheDocument();
  });

  it("reports a refusal without looking broken", async () => {
    mockConnect.mockResolvedValue({ connected: false, reason: "access_denied" });
    render(<GoogleCalendarCard />);

    fireEvent.click(
      await screen.findByRole("button", { name: /connect google calendar/i }),
    );
    expect(await screen.findByText(/You declined access/)).toBeInTheDocument();
  });

  it("offers detection only once connected", async () => {
    // Switching the source on before there is a token would do nothing and
    // look broken.
    render(<GoogleCalendarCard />);
    await screen.findByRole("button", { name: /connect google calendar/i });
    expect(screen.queryByLabelText(/use google calendar for detection/i)).toBeNull();
  });

  it("can turn the source on once connected", async () => {
    mockGet.mockResolvedValue(settings({ connected: true }));
    render(<GoogleCalendarCard />);

    fireEvent.click(await screen.findByLabelText(/use google calendar for detection/i));
    await waitFor(() => expect(mockSetEnabled).toHaveBeenCalledWith(true));
  });

  it("disconnects and says the token was deleted", async () => {
    mockGet.mockResolvedValue(settings({ connected: true }));
    render(<GoogleCalendarCard />);

    fireEvent.click(await screen.findByRole("button", { name: /disconnect/i }));
    await waitFor(() => expect(mockDisconnect).toHaveBeenCalled());
    expect(await screen.findByText(/token was deleted/i)).toBeInTheDocument();
  });
});
