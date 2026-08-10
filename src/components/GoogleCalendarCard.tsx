import { useCallback, useEffect, useState } from "react";
import {
  gcalConnect,
  gcalDisconnect,
  gcalSetClientId,
  gcalSetClientSecret,
  gcalSettings,
} from "../lib/tauri";
import type { GcalSettings } from "../types";

/** Whether a string looks like a Google OAuth client id. */
export function looksLikeClientId(value: string): boolean {
  // Not validation for its own sake: the two fields below take two long opaque
  // strings, and swapping them fails much later with an opaque Google error.
  // A client id always ends in this suffix.
  return value.trim().endsWith(".apps.googleusercontent.com");
}

/** Whether a string looks like a Google OAuth client secret. */
export function looksLikeClientSecret(value: string): boolean {
  // Google prefixes them `GOCSPX-`. Same reason as above, in the other
  // direction: a client id pasted here is caught before Google sees it.
  return value.trim().startsWith("GOCSPX-");
}

/**
 * Which half of the credential is still missing.
 *
 * Named rather than a generic "fill in the fields": the two are saved
 * separately and the secret's field is blank even once it is stored, so
 * "something is missing" leaves the user staring at a form that looks done.
 */
export function missingHalf(settings: {
  clientId: string | null;
  hasClientSecret: boolean;
}): string {
  const missing = [
    settings.clientId ? null : "client id",
    settings.hasClientSecret ? null : "client secret",
  ].filter(Boolean);
  if (missing.length === 0) return "";
  return `Save the ${missing.join(" and the ")} first.`;
}

/** What to say about a finished flow. */
export function connectMessage(connected: boolean, reason: string | null): string {
  if (connected) return "Connected. Oatmeal can read your upcoming events.";
  if (!reason) return "Not connected.";
  if (reason.includes("access_denied")) {
    return "You declined access, so nothing was connected.";
  }
  if (reason.includes("mismatched state")) {
    return "The browser came back with something Oatmeal did not start. Nothing was connected.";
  }
  if (reason.includes("invalid_request")) {
    // What Google says when the credential is incomplete or mismatched. The
    // bare word is useless: it arrives *after* consent, so the user has every
    // reason to think they did their part — and they did.
    return (
      "Google rejected the credential. Check that the client id and secret are from " +
      "the same OAuth client, and that it is a Desktop app client."
    );
  }
  return reason;
}

/**
 * Google Calendar over OAuth.
 *
 * An *addition* to the macOS Calendar source, not a replacement: if the account
 * is already in Calendar.app, EventKit reads it with no accounts and no tokens,
 * and this is unnecessary. It exists for calendars macOS does not sync.
 *
 * Both halves of the credential are the user's own, created in their own Google
 * Cloud project. Nothing confidential ships in the binary.
 *
 * The secret **is** required, which this card used to say it wasn't. Google
 * documents it as non-confidential for installed apps — true, and not the same
 * as optional. Its token endpoint answers a request without it with
 * `invalid_request: client_secret is missing.` PKCE is still what does the real
 * work, since a secret shipped in a desktop binary protects nothing.
 */
export function GoogleCalendarCard() {
  const [settings, setSettings] = useState<GcalSettings | null>(null);
  const [clientId, setClientId] = useState("");
  /* Never seeded from settings — the secret is write-only. What comes back is
     whether one is stored, not what it is. */
  const [clientSecret, setClientSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await gcalSettings();
      setSettings(next);
      setClientId(next.clientId ?? "");
    } catch (err) {
      setMessage(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function saveClientId() {
    await gcalSetClientId(clientId);
    await refresh();
  }

  async function saveClientSecret() {
    await gcalSetClientSecret(clientSecret);
    // Cleared on save: leaving the secret sitting in a form field is one more
    // place it exists for no benefit.
    setClientSecret("");
    await refresh();
  }

  async function connect() {
    setBusy(true);
    setMessage("Waiting for your browser…");
    try {
      const outcome = await gcalConnect();
      setMessage(connectMessage(outcome.connected, outcome.reason));
      await refresh();
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function disconnect() {
    await gcalDisconnect();
    setMessage("Disconnected. The stored token was deleted.");
    await refresh();
  }

  if (!settings) {
    return (
      <section className="card">
        <div className="card-head">
          <h2>Google Calendar</h2>
        </div>
        <p className="empty-note">{message ?? "Loading…"}</p>
      </section>
    );
  }

  const idLooksWrong = clientId.trim() !== "" && !looksLikeClientId(clientId);
  const secretLooksWrong =
    clientSecret.trim() !== "" && !looksLikeClientSecret(clientSecret);
  /* Both halves, or Connect fails at the token exchange after the user has
     already been through consent — the worst possible place to find out. */
  const ready = Boolean(settings.clientId) && settings.hasClientSecret;

  return (
    <section className="card" data-testid="gcal-card">
      <div className="card-head">
        <h2>Google Calendar</h2>
        {settings.connected && <span className="pill pill--ok">connected</span>}
      </div>
      <p className="card-note">
        Only needed if your calendar is <em>not</em> in the macOS Calendar app — Oatmeal
        already reads that one, with no account and no tokens. Read-only, and the token
        is stored in your Keychain.
      </p>

      <div className="row">
        <input
          value={clientId}
          placeholder="…apps.googleusercontent.com"
          aria-label="google oauth client id"
          onChange={(e) => setClientId(e.target.value)}
          style={{ flex: 1 }}
        />
        <button onClick={() => void saveClientId()}>Save</button>
      </div>
      {idLooksWrong && (
        <p className="empty-note">
          That does not look like a client <em>id</em> — it should end in
          <code> .apps.googleusercontent.com</code>. The secret goes in the field below.
        </p>
      )}

      <div className="row">
        <input
          type="password"
          value={clientSecret}
          placeholder={settings.hasClientSecret ? "client secret — stored" : "GOCSPX-…"}
          aria-label="google oauth client secret"
          onChange={(e) => setClientSecret(e.target.value)}
          style={{ flex: 1 }}
        />
        <button onClick={() => void saveClientSecret()}>Save</button>
      </div>
      {secretLooksWrong && (
        <p className="empty-note">
          That does not look like a client <em>secret</em> — Google prefixes them
          <code> GOCSPX-</code>.
        </p>
      )}

      <p className="empty-note">
        Create a <strong>Desktop app</strong> OAuth client in Google Cloud Console and
        paste both halves. Google requires the secret here even though PKCE is in use;
        it goes to your Keychain, never to a file, and is never shown again.
      </p>

      <div className="row">
        {settings.connected ? (
          <button onClick={() => void disconnect()}>Disconnect</button>
        ) : (
          <button
            className="primary"
            disabled={busy || !ready}
            onClick={() => void connect()}
          >
            {busy ? "Waiting for browser…" : "Connect Google Calendar"}
          </button>
        )}
        {!ready && <span className="empty-note">{missingHalf(settings)}</span>}
      </div>

      {/* The "use these events" switch lives in Visible calendars now, with the
          other calendars. Two controls for one setting is how a user learns not
          to trust either of them. */}

      {message && <p className="empty-note">{message}</p>}
    </section>
  );
}
