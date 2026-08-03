import { useCallback, useEffect, useState } from "react";
import {
  notionDatabases,
  notionExport,
  notionSetOptions,
  notionSetToken,
  notionSettings,
} from "../lib/tauri";
import type { NotionDatabase, NotionSettings } from "../types";

interface Props {
  /** The meeting to export, when one is open. */
  meetingId?: string | null;
}

/** What an export did, in words. */
export function exportSummary(created: boolean, blocks: number): string {
  const action = created ? "Created a page" : "Updated the existing page";
  return `${action} with ${blocks} block${blocks === 1 ? "" : "s"}.`;
}

/**
 * Sending a meeting to Notion.
 *
 * One page per meeting in a database the user picks — the documented default
 * for the export-shape gate. The page id is remembered, so exporting a second
 * time after regenerating a summary updates that page rather than leaving a
 * trail of near-duplicates.
 */
export function NotionCard({ meetingId }: Props) {
  const [settings, setSettings] = useState<NotionSettings | null>(null);
  const [databases, setDatabases] = useState<NotionDatabase[]>([]);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSettings(await notionSettings());
    } catch (err) {
      setMessage(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const loadDatabases = useCallback(async () => {
    setBusy(true);
    setMessage(null);
    try {
      setDatabases(await notionDatabases());
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }, []);

  async function saveToken() {
    await notionSetToken(token);
    setToken("");
    await refresh();
    if (token.trim()) await loadDatabases();
  }

  async function setOptions(next: Partial<NotionSettings>) {
    if (!settings) return;
    const merged = { ...settings, ...next };
    setSettings(merged);
    await notionSetOptions(
      merged.databaseId,
      merged.includeTranscript,
      merged.autoExport,
    );
  }

  async function exportNow() {
    if (!meetingId) return;
    setBusy(true);
    setMessage(null);
    try {
      const result = await notionExport(meetingId);
      setMessage(exportSummary(result.created, result.blocks));
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="card" data-testid="notion-card">
      <div className="card-head">
        <h2>Notion</h2>
        {settings?.hasToken && <span className="pill pill--ok">connected</span>}
      </div>
      <p className="card-note">
        One page per meeting, in a database you choose. Exporting again after
        regenerating a summary updates that page instead of making another.
      </p>

      <div className="row">
        <input
          type="password"
          value={token}
          placeholder={
            settings?.hasToken ? "token stored — paste a new one to replace" : "ntn_…"
          }
          aria-label="notion integration token"
          onChange={(e) => setToken(e.target.value)}
          style={{ flex: 1 }}
        />
        <button onClick={() => void saveToken()}>
          {token ? "Save token" : "Remove token"}
        </button>
      </div>
      <p className="empty-note">
        Create an integration at notion.so/my-integrations, then share the target
        database with it — Notion shows an integration only what you share.
      </p>

      {settings?.hasToken && (
        <>
          <div className="row">
            <button onClick={() => void loadDatabases()} disabled={busy}>
              {busy ? "Loading…" : "Find databases"}
            </button>
            <select
              aria-label="notion database"
              value={settings.databaseId ?? ""}
              onChange={(e) => void setOptions({ databaseId: e.target.value || null })}
            >
              <option value="">Choose a database…</option>
              {databases.map((database) => (
                <option key={database.id} value={database.id}>
                  {database.title}
                </option>
              ))}
            </select>
          </div>

          {databases.length === 0 && !busy && (
            // Empty is a normal state, not an error: Notion only returns what
            // has been shared. Saying so beats an empty dropdown.
            <p className="empty-note">
              No databases yet. Open the database in Notion, then ⋯ → Connections → add
              your integration.
            </p>
          )}

          <label className="tuner-row">
            <span>Include the full transcript</span>
            <input
              type="checkbox"
              aria-label="include the full transcript"
              checked={settings.includeTranscript}
              onChange={(e) => void setOptions({ includeTranscript: e.target.checked })}
            />
          </label>

          <label className="tuner-row">
            <span>Export automatically when a meeting ends</span>
            <input
              type="checkbox"
              aria-label="export automatically"
              checked={settings.autoExport}
              onChange={(e) => void setOptions({ autoExport: e.target.checked })}
            />
          </label>

          <div className="row">
            <button
              className="primary"
              disabled={busy || !meetingId || !settings.databaseId}
              onClick={() => void exportNow()}
            >
              {busy ? "Exporting…" : "Export this meeting"}
            </button>
            {!meetingId && <span className="empty-note">Open a meeting first.</span>}
            {meetingId && !settings.databaseId && (
              <span className="empty-note">Choose a database first.</span>
            )}
          </div>
        </>
      )}

      {message && <p className="empty-note">{message}</p>}
    </section>
  );
}
