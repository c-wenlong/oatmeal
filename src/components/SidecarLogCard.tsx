import { useCallback, useEffect, useState } from "react";
import { sidecarLogPath, sidecarLogTail } from "../lib/tauri";

/**
 * The sidecar log, where a user can reach it.
 *
 * The sidecar's stderr is the only account of what audio capture, the ASR model
 * and the calendar watcher actually did. In a bundled `.app` launched from
 * Finder there is no terminal attached, so all of it went nowhere — every
 * failure in there was invisible unless the app happened to be started from a
 * shell.
 *
 * This is a diagnostic and it is framed as one. It sits under About rather than
 * pretending to be a setting, it says nothing when there is nothing, and it
 * offers the file path so a log can be attached to a bug report rather than
 * retyped from a screenshot.
 */

/** The interesting lines, when the whole log is too much to read. */
export function calendarLines(lines: string[]): string[] {
  return lines.filter((line) => /\[calendar\]|Calendar/i.test(line));
}

/** Strips the leading timestamp for display, keeping the line itself. */
export function withoutStamp(line: string): string {
  return line.replace(/^\d{10,}\s/, "");
}

export function SidecarLogCard() {
  const [lines, setLines] = useState<string[] | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [calendarOnly, setCalendarOnly] = useState(false);

  const load = useCallback(async () => {
    try {
      const [tail, where] = await Promise.all([sidecarLogTail(200), sidecarLogPath()]);
      setLines(tail);
      setPath(where);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const shown = lines ? (calendarOnly ? calendarLines(lines) : lines) : [];

  return (
    <section className="card" data-testid="sidecar-log">
      <div className="card-head">
        <h2>Sidecar log</h2>
      </div>
      <p className="card-note">
        What the recorder, the speech model and the calendar watcher reported. Useful
        when something is empty or silent and it is not obvious why.
      </p>

      <div className="row">
        <button onClick={() => void load()}>Refresh</button>
        <button onClick={() => setCalendarOnly((was) => !was)}>
          {calendarOnly ? "Show everything" : "Calendar only"}
        </button>
      </div>

      {error && <p className="empty-note">{error}</p>}

      {lines && lines.length === 0 && (
        <p className="empty-note">
          Nothing logged yet. The sidecar writes here once it starts, which happens when
          detection is on or a recording begins.
        </p>
      )}

      {lines && lines.length > 0 && shown.length === 0 && (
        <p className="empty-note">
          No calendar lines in the last {lines.length} entries.
        </p>
      )}

      {shown.length > 0 && (
        <div className="log log--tail">
          {shown.map((line, i) => (
            <div className="log-line" key={i}>
              <span className="log-text">{withoutStamp(line)}</span>
            </div>
          ))}
        </div>
      )}

      {path && <p className="empty-note log-path">{path}</p>}
    </section>
  );
}
