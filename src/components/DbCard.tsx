import { useState } from "react";
import { dbSelftest } from "../lib/tauri";
import type { DbSelftest } from "../types";

type State =
  | { status: "idle" }
  | { status: "running" }
  | { status: "ok"; result: DbSelftest }
  | { status: "error"; message: string };

/**
 * The self-test writes to a scratch in-memory database, so it can be run
 * repeatedly without touching real meetings. It checks the two things most
 * likely to differ between SQLite builds: FTS5 Porter stemming and the
 * sqlite-vec extension.
 */
export function DbCard() {
  const [state, setState] = useState<State>({ status: "idle" });

  async function run() {
    setState({ status: "running" });
    try {
      setState({ status: "ok", result: await dbSelftest() });
    } catch (err: unknown) {
      setState({ status: "error", message: String(err) });
    }
  }

  const stemmingOk = state.status === "ok" && state.result.ftsHit !== null;
  const vectorOk = state.status === "ok" && state.result.vectorHit === "near";
  const allOk = stemmingOk && vectorOk;

  return (
    <section className="card">
      <div className="card-head">
        <h2>Data layer</h2>
        {state.status === "idle" && <span className="pill pill--pending">not run</span>}
        {state.status === "running" && (
          <span className="pill pill--pending">running</span>
        )}
        {state.status === "ok" && (
          <span className={allOk ? "pill pill--ok" : "pill pill--err"}>
            {allOk ? "passing" : "degraded"}
          </span>
        )}
        {state.status === "error" && <span className="pill pill--err">failed</span>}
      </div>
      <p className="card-note">
        Migrates a scratch database, then round-trips a meeting through full-text and
        vector search. Nothing here touches your real meetings.
      </p>

      <div className="row">
        <button className="primary" onClick={run} disabled={state.status === "running"}>
          {state.status === "idle" ? "Run self-test" : "Run again"}
        </button>
      </div>

      {state.status === "ok" && (
        <dl className="kv" style={{ marginTop: 16 }}>
          <dt>Schema version</dt>
          <dd>{state.result.schemaVersion}</dd>
          <dt>FTS5 stemming</dt>
          <dd>
            {stemmingOk
              ? `"migrate" matched "${state.result.ftsHit}"`
              : "FAILED — no hit for stemmed query"}
          </dd>
          <dt>Vector search</dt>
          <dd>
            {vectorOk
              ? `nearest = "${state.result.vectorHit}"`
              : `FAILED — got "${state.result.vectorHit ?? "nothing"}"`}
          </dd>
          <dt>Your database</dt>
          <dd>{state.result.dbPath}</dd>
          <dt>Stored rows</dt>
          <dd>
            {state.result.stats.meetings} meetings &middot;{" "}
            {state.result.stats.utterances} utterances &middot;{" "}
            {state.result.stats.noteBlocks} notes &middot; {state.result.stats.panels}{" "}
            panels
          </dd>
        </dl>
      )}

      {state.status === "error" && <p className="empty-note">{state.message}</p>}
    </section>
  );
}
