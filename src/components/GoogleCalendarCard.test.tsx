import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  GoogleCalendarCard,
  connectMessage,
  looksLikeClientId,
  looksLikeClientSecret,
  missingHalf,
} from "./GoogleCalendarCard";
import {
  gcalConnect,
  gcalDisconnect,
  gcalSetClientId,
  gcalSetClientSecret,
  gcalSetEnabled,
  gcalSettings,
} from "../lib/tauri";
import type { GcalSettings } from "../types";

vi.mock("../lib/tauri", () => ({
  gcalSettings: vi.fn(),
  gcalSetClientId: vi.fn(),
  gcalSetClientSecret: vi.fn(),
  gcalSetEnabled: vi.fn(),
  gcalConnect: vi.fn(),
  gcalDisconnect: vi.fn(),
}));

const mockGet = vi.mocked(gcalSettings);
const mockSetId = vi.mocked(gcalSetClientId);
const mockSetSecret = vi.mocked(gcalSetClientSecret);
const mockSetEnabled = vi.mocked(gcalSetEnabled);
const mockConnect = vi.mocked(gcalConnect);
const mockDisconnect = vi.mocked(gcalDisconnect);

const CLIENT_ID = "123-abc.apps.googleusercontent.com";

function settings(over: Partial<GcalSettings> = {}): GcalSettings {
  return {
    connected: false,
    clientId: CLIENT_ID,
    hasClientSecret: true,
    enabled: false,
    ...over,
  };
}

beforeEach(() => {
  for (const m of [
    mockGet,
    mockSetId,
    mockSetSecret,
    mockSetEnabled,
    mockConnect,
    mockDisconnect,
  ]) {
    m.mockReset();
  }
  mockGet.mockResolvedValue(settings());
  mockSetId.mockResolvedValue(undefined);
  mockSetSecret.mockResolvedValue(undefined);
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

  it("says the secret is required, and where it goes", async () => {
    // This card used to say the opposite. Google requires client_secret for
    // Desktop app clients whether or not PKCE is used, and the promise that it
    // was unnecessary made the Connect button impossible to succeed at.
    render(<GoogleCalendarCard />);
    expect(
      await screen.findByText(/Google requires the secret here/i),
    ).toBeInTheDocument();
    expect(screen.getByText(/goes to your Keychain/i)).toBeInTheDocument();
  });

  it("warns when an id was pasted into the secret field", async () => {
    // The two fields take two long opaque strings. Swapped, they fail at the
    // token exchange — after consent, with an opaque Google error.
    render(<GoogleCalendarCard />);
    fireEvent.change(await screen.findByLabelText(/google oauth client secret/i), {
      target: { value: CLIENT_ID },
    });
    expect(screen.getByText(/does not look like a client/i)).toBeInTheDocument();
  });

  it("saves a client secret and does not keep it in the field", async () => {
    render(<GoogleCalendarCard />);
    const field = await screen.findByLabelText(/google oauth client secret/i);
    fireEvent.change(field, { target: { value: "GOCSPX-shh" } });
    fireEvent.click(screen.getAllByRole("button", { name: /save/i })[1]);

    await waitFor(() => expect(mockSetSecret).toHaveBeenCalledWith("GOCSPX-shh"));
    // Left in the form it would exist in one more place for no benefit.
    await waitFor(() => expect(field).toHaveValue(""));
  });

  it("never renders the stored secret", async () => {
    // It is write-only: the backend reports that one is set, not what it is.
    mockGet.mockResolvedValue(settings({ hasClientSecret: true }));
    render(<GoogleCalendarCard />);
    const field = await screen.findByLabelText(/google oauth client secret/i);
    expect(field).toHaveValue("");
    expect(field).toHaveAttribute("type", "password");
  });

  it("will not connect with only half the credential", async () => {
    // Both halves or nothing: Google fails the exchange *after* consent, which
    // is the worst possible place for the user to learn a field was blank.
    mockGet.mockResolvedValue(settings({ hasClientSecret: false }));
    render(<GoogleCalendarCard />);

    expect(
      await screen.findByRole("button", { name: /connect google calendar/i }),
    ).toBeDisabled();
    expect(screen.getByText(/Save the client secret first/)).toBeInTheDocument();
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
    // Two Save buttons now — the id's is the first.
    fireEvent.click(screen.getAllByRole("button", { name: /save/i })[0]);

    await waitFor(() => expect(mockSetId).toHaveBeenCalledWith(CLIENT_ID));
  });

  it("will not connect without a client id", async () => {
    mockGet.mockResolvedValue(settings({ clientId: null }));
    render(<GoogleCalendarCard />);

    expect(
      await screen.findByRole("button", { name: /connect google calendar/i }),
    ).toBeDisabled();
    expect(screen.getByText(/Save the client id first/)).toBeInTheDocument();
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

  it("does not own the use-for-detection switch", async () => {
    // It lives in Visible calendars now, with the other calendars. Two
    // controls for one setting is how a user learns to trust neither — and
    // this card is behind a disclosure most people never open.
    mockGet.mockResolvedValue(settings({ connected: true }));
    render(<GoogleCalendarCard />);

    await screen.findByRole("button", { name: /disconnect/i });
    expect(screen.queryByLabelText(/use google calendar for detection/i)).toBeNull();
  });

  it("disconnects and says the token was deleted", async () => {
    mockGet.mockResolvedValue(settings({ connected: true }));
    render(<GoogleCalendarCard />);

    fireEvent.click(await screen.findByRole("button", { name: /disconnect/i }));
    await waitFor(() => expect(mockDisconnect).toHaveBeenCalled());
    expect(await screen.findByText(/token was deleted/i)).toBeInTheDocument();
  });
});

describe("missingHalf", () => {
  it("names which half is missing, not just that something is", () => {
    // The secret's field is blank even when one is stored, so "fill in the
    // fields" leaves the user staring at a form that looks finished.
    expect(missingHalf({ clientId: null, hasClientSecret: true })).toBe(
      "Save the client id first.",
    );
    expect(missingHalf({ clientId: "x", hasClientSecret: false })).toBe(
      "Save the client secret first.",
    );
    expect(missingHalf({ clientId: null, hasClientSecret: false })).toBe(
      "Save the client id and the client secret first.",
    );
  });

  it("says nothing when both are there", () => {
    expect(missingHalf({ clientId: "x", hasClientSecret: true })).toBe("");
  });
});

describe("looksLikeClientSecret", () => {
  it("knows Google's prefix", () => {
    expect(looksLikeClientSecret("GOCSPX-abc")).toBe(true);
    expect(looksLikeClientSecret("  GOCSPX-abc  ")).toBe(true);
  });

  it("rejects a client id", () => {
    expect(looksLikeClientSecret("123-abc.apps.googleusercontent.com")).toBe(false);
  });
});
