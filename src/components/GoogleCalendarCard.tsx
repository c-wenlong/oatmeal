import { useCallback, useEffect, useState } from "react";
import {
  gcalConnect,
  gcalDisconnect,
  gcalSetClientId,
  gcalSetEnabled,
  gcalSettings,
} from "../lib/tauri";
import type { GcalSettings } from "../types";

/** Whether a string looks like a Google OAuth client id. */
export function looksLikeClientId(value: string): boolean {
  // Not validation for its own sake: pasting the *secret* into this field is
  // the mistake people actually make, and it fails much later with an opaque
  // Google error. A client id always ends in this suffix.
  return value.trim().endsWith(".apps.googleusercontent.com");
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
  return reason;
}

/**
 * Google Calendar over OAuth.
 *
 * An *addition* to the macOS Calendar source, not a replacement: if the account
 * is already in Calendar.app, EventKit reads it with no accounts and no tokens,
 * and this is unnecessary. It exists for calendars macOS does not sync.
 *
 * No client secret is involved. Google treats installed apps as public clients,
 * and PKCE does the job the secret used to — so nothing confidential ships in
 * the binary, and the client id below is the user's own.
 */
export function GoogleCalendarCard() {
  const [settings, setSettings] = useState<GcalSettings | null>(null);
  const [clientId, setClientId] = useState("");
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
          <code> .apps.googleusercontent.com</code>. Oatmeal never needs the client
          secret.
        </p>
      )}
      <p className="empty-note">
        Create a <strong>Desktop app</strong> OAuth client in Google Cloud Console and
        paste its id. There is no secret to copy: Oatmeal uses PKCE, so nothing
        confidential is stored in the app.
      </p>

      <div className="row">
        {settings.connected ? (
          <button onClick={() => void disconnect()}>Disconnect</button>
        ) : (
          <button
            className="primary"
            disabled={busy || !settings.clientId}
            onClick={() => void connect()}
          >
            {busy ? "Waiting for browser…" : "Connect Google Calendar"}
          </button>
        )}
        {!settings.clientId && (
          <span className="empty-note">Save a client id first.</span>
        )}
      </div>

      {settings.connected && (
        <label className="tuner-row">
          <span>Use these events for meeting detection</span>
          <input
            type="checkbox"
            aria-label="use google calendar for detection"
            checked={settings.enabled}
            onChange={(e) => void gcalSetEnabled(e.target.checked).then(refresh)}
          />
        </label>
      )}

      {message && <p className="empty-note">{message}</p>}
    </section>
  );
}
