import { useCallback, useEffect, useRef, useState } from "react";
import {
  folderCreate,
  folderDelete,
  folderMeetings,
  foldersList,
  meetingSetFolder,
  searchTranscripts,
} from "../lib/tauri";
import { matchLabel, segments, timecode } from "../lib/highlight";
import type { Folder, MeetingSummary, SearchResponse } from "../types";

interface Props {
  /** Opens a meeting and scrolls to a moment in it. */
  onReveal?: (meetingId: string, utteranceId: number) => void;
}

/**
 * Finding a conversation again.
 *
 * The goal this serves is "a phrase you remember imperfectly from three weeks
 * ago", so results are grouped by meeting rather than listed flat — the real
 * question is usually *which conversation was that in*, and five hits from one
 * long call would otherwise bury five different calls that each matched once.
 */
export function SearchCard({ onReveal }: Props) {
  const [query, setQuery] = useState("");
  const [folders, setFolders] = useState<Folder[]>([]);
  const [scope, setScope] = useState<string | null>(null);
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [filed, setFiled] = useState<MeetingSummary[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Guards against an older search landing after a newer one and overwriting
  // it — with a local model in the loop, latency varies enough for this to
  // happen on ordinary typing.
  const generation = useRef(0);

  const refreshFolders = useCallback(async () => {
    try {
      setFolders(await foldersList());
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refreshFolders();
  }, [refreshFolders]);

  useEffect(() => {
    void folderMeetings(scope)
      .then(setFiled)
      .catch(() => setFiled([]));
  }, [scope, response]);

  const run = useCallback(async (text: string, folderId: string | null) => {
    const ticket = ++generation.current;
    if (!text.trim()) {
      setResponse(null);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const next = await searchTranscripts(text, folderId);
      if (ticket === generation.current) {
        setResponse(next);
      }
    } catch (err) {
      if (ticket === generation.current) setError(String(err));
    } finally {
      if (ticket === generation.current) setBusy(false);
    }
  }, []);

  // Debounced: every keystroke would otherwise embed the query, which is a
  // round trip to a local model.
  useEffect(() => {
    const timer = setTimeout(() => void run(query, scope), 250);
    return () => clearTimeout(timer);
  }, [query, scope, run]);

  async function addFolder() {
    const name = window.prompt("Folder name");
    if (!name?.trim()) return;
    await folderCreate(name);
    await refreshFolders();
  }

  async function removeFolder(folder: Folder) {
    // Said explicitly because "delete folder" reads like it might take the
    // recordings with it. It does not — the schema makes sure of that.
    if (
      !window.confirm(
        `Delete "${folder.name}"? Its ${folder.meetingCount} meeting(s) are kept and become unfiled.`,
      )
    ) {
      return;
    }
    await folderDelete(folder.id);
    if (scope === folder.id) setScope(null);
    await refreshFolders();
  }

  return (
    <section className="card" data-testid="search-card">
      <div className="card-head">
        <h2>Search</h2>
        {busy && <span className="pill pill--pending">searching</span>}
      </div>
      <p className="card-note">
        Looks for the words you type <em>and</em> for what you meant, then groups what
        it finds by meeting.
      </p>

      <div className="row">
        <input
          type="search"
          value={query}
          placeholder="a phrase you half remember…"
          aria-label="search transcripts"
          onChange={(e) => setQuery(e.target.value)}
          style={{ flex: 1 }}
        />
      </div>

      <div className="row folder-bar">
        <button
          className={scope === null ? "chip chip--on" : "chip"}
          onClick={() => setScope(null)}
        >
          All meetings
        </button>
        {folders.map((folder) => (
          <span key={folder.id} className="chip-group">
            <button
              className={scope === folder.id ? "chip chip--on" : "chip"}
              onClick={() => setScope(folder.id)}
            >
              {folder.name} ({folder.meetingCount})
            </button>
            <button
              className="link-button"
              aria-label={`delete folder ${folder.name}`}
              onClick={() => void removeFolder(folder)}
            >
              ×
            </button>
          </span>
        ))}
        <button className="link-button" onClick={() => void addFolder()}>
          New folder
        </button>
      </div>

      {response && !response.semantic && response.results.length > 0 && (
        <p className="empty-note">
          Words only — no embedding model is reachable, so this did not search by
          meaning.
        </p>
      )}

      {response?.results.length === 0 && query.trim() && !busy && (
        <p className="empty-note">Nothing matched “{query}”.</p>
      )}

      <ul className="results">
        {response?.results.map((result) => (
          <li key={result.meetingId} className="result">
            <div className="result-head">
              <button
                className="link-button"
                onClick={() => onReveal?.(result.meetingId, result.bestUtteranceId)}
              >
                {result.title?.trim() || "Untitled meeting"}
              </button>
              <span className="empty-note">
                {result.hits.length} match{result.hits.length === 1 ? "" : "es"}
              </span>
            </div>
            <ul className="result-hits">
              {result.hits.map((hit, index) => {
                const snippet = result.previews[index];
                return (
                  <li key={hit.utteranceId}>
                    <button
                      className="result-time"
                      title="Jump to this moment"
                      onClick={() => onReveal?.(hit.meetingId, hit.utteranceId)}
                    >
                      {timecode(hit.startMs)}
                    </button>
                    <span className="result-text">
                      {snippet?.truncatedStart && "…"}
                      {snippet
                        ? segments(snippet).map((segment, position) =>
                            segment.marked ? (
                              <mark key={position}>{segment.text}</mark>
                            ) : (
                              <span key={position}>{segment.text}</span>
                            ),
                          )
                        : hit.text}
                      {snippet?.truncatedEnd && "…"}
                    </span>
                    <span className="empty-note result-why">
                      {matchLabel(hit.kind)}
                    </span>
                  </li>
                );
              })}
            </ul>
          </li>
        ))}
      </ul>

      {!query.trim() && (
        <div className="filed">
          <p className="empty-note">
            {scope === null
              ? "Unfiled meetings"
              : `In ${folders.find((f) => f.id === scope)?.name ?? "this folder"}`}
          </p>
          <ul className="rule-list" data-testid="filed-meetings">
            {filed.map((meeting) => (
              <li key={meeting.id}>
                <span>{meeting.title?.trim() || "Untitled meeting"}</span>
                <select
                  aria-label={`folder for ${meeting.title ?? meeting.id}`}
                  value={scope ?? ""}
                  onChange={(e) => {
                    const next = e.target.value || null;
                    void meetingSetFolder(meeting.id, next).then(() => {
                      void refreshFolders();
                      void folderMeetings(scope).then(setFiled);
                    });
                  }}
                >
                  <option value="">Unfiled</option>
                  {folders.map((folder) => (
                    <option key={folder.id} value={folder.id}>
                      {folder.name}
                    </option>
                  ))}
                </select>
              </li>
            ))}
            {filed.length === 0 && <li className="empty-note">Nothing here yet.</li>}
          </ul>
        </div>
      )}

      {error && <p className="empty-note">{error}</p>}
    </section>
  );
}
