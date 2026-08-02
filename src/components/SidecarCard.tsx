import { useCallback, useEffect, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  onSidecarEvent,
  sidecarSend,
  sidecarSimulateCrash,
  sidecarStart,
  sidecarStop,
} from "../lib/tauri";
import { elapsedLabel, formatSupervisorEvent, type LogLine } from "../lib/sidecarLog";
import type { SupervisorEvent } from "../types";

interface Entry extends LogLine {
  id: number;
  at: string;
}

type Status = "stopped" | "starting" | "running" | "error";

/** Keeps the log bounded so a long session can't grow the DOM without limit. */
const MAX_ENTRIES = 200;

export function SidecarCard() {
  const [status, setStatus] = useState<Status>("stopped");
  const [binaryPath, setBinaryPath] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [entries, setEntries] = useState<Entry[]>([]);

  const startedAt = useRef(Date.now());
  const nextId = useRef(0);
  const unlisten = useRef<UnlistenFn | null>(null);
  const logRef = useRef<HTMLDivElement | null>(null);

  const append = useCallback((event: SupervisorEvent) => {
    const line = formatSupervisorEvent(event);
    setEntries((previous) => {
      const entry: Entry = {
        ...line,
        id: nextId.current++,
        at: elapsedLabel(startedAt.current, Date.now()),
      };
      return [...previous, entry].slice(-MAX_ENTRIES);
    });

    if (event.kind === "gave_up") setStatus("error");
    if (event.kind === "event" && event.event.ev === "ready") setStatus("running");
  }, []);

  useEffect(() => {
    let cancelled = false;
    onSidecarEvent(append).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten.current = fn;
      }
    });
    return () => {
      cancelled = true;
      unlisten.current?.();
      unlisten.current = null;
    };
  }, [append]);

  // Follow the tail as events arrive.
  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [entries]);

  async function guard(action: () => Promise<void>) {
    setMessage(null);
    try {
      await action();
    } catch (err: unknown) {
      setMessage(String(err));
    }
  }

  const start = () =>
    guard(async () => {
      setStatus("starting");
      startedAt.current = Date.now();
      setEntries([]);
      try {
        setBinaryPath(await sidecarStart());
      } catch (err) {
        setStatus("error");
        throw err;
      }
    });

  const stop = () =>
    guard(async () => {
      await sidecarStop();
      setStatus("stopped");
    });

  const running = status === "running" || status === "starting";

  return (
    <section className="card">
      <div className="card-head">
        <h2>Sidecar</h2>
        {status === "stopped" && <span className="pill pill--pending">stopped</span>}
        {status === "starting" && <span className="pill pill--pending">starting</span>}
        {status === "running" && <span className="pill pill--ok">connected</span>}
        {status === "error" && <span className="pill pill--err">failed</span>}
      </div>
      <p className="card-note">
        Spawns the Swift sidecar and streams its events over stdio. The transcript below
        is a scripted fixture &mdash; real audio arrives in G6/G7. Kill it to watch the
        supervisor bring it back.
      </p>

      <div className="row">
        <button className="primary" onClick={start} disabled={running}>
          Start sidecar
        </button>
        <button onClick={stop} disabled={!running}>
          Stop
        </button>
        <button
          onClick={() =>
            guard(() =>
              sidecarSend({
                cmd: "start",
                meeting_id: "harness",
                sources: ["mic", "system"],
              }),
            )
          }
          disabled={status !== "running"}
        >
          Run session
        </button>
        <button
          onClick={() => guard(() => sidecarSend({ cmd: "ping" }))}
          disabled={status !== "running"}
        >
          Ping
        </button>
        <button
          onClick={() => guard(() => sidecarSimulateCrash())}
          disabled={status !== "running"}
        >
          Simulate crash
        </button>
      </div>

      {binaryPath && (
        <dl className="kv" style={{ marginTop: 16 }}>
          <dt>Binary</dt>
          <dd>{binaryPath}</dd>
        </dl>
      )}

      {message && <p className="empty-note">{message}</p>}

      <div className="log" ref={logRef} data-testid="sidecar-log">
        {entries.map((entry) => (
          <div className="log-line" key={entry.id}>
            <span className="log-time">{entry.at}</span>
            <span className={`log-tag log-tag--${entry.tone}`}>{entry.tag}</span>
            <span className={entry.partial ? "log-text log-partial" : "log-text"}>
              {entry.text}
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}
