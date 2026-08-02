import { useEffect, useState } from "react";
import { healthCheck } from "../lib/tauri";
import type { HealthInfo } from "../types";

type State =
  | { status: "loading" }
  | { status: "ok"; info: HealthInfo }
  | { status: "error"; message: string };

/**
 * Proves the React -> Rust bridge is live. If this card is green, Tauri IPC
 * works; every later card can assume it.
 */
export function HealthCard() {
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;
    healthCheck()
      .then((info) => {
        if (!cancelled) setState({ status: "ok", info });
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setState({ status: "error", message: String(err) });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <section className="card">
      <div className="card-head">
        <h2>Rust core</h2>
        {state.status === "loading" && (
          <span className="pill pill--pending">checking</span>
        )}
        {state.status === "ok" && <span className="pill pill--ok">connected</span>}
        {state.status === "error" && (
          <span className="pill pill--err">unreachable</span>
        )}
      </div>
      <p className="card-note">
        Round-trips a command over Tauri IPC. Green means the frontend and the Rust core
        are talking.
      </p>

      {state.status === "ok" && (
        <dl className="kv">
          <dt>App version</dt>
          <dd>{state.info.appVersion}</dd>
          <dt>Build profile</dt>
          <dd>{state.info.buildProfile}</dd>
          <dt>Target</dt>
          <dd>
            {state.info.os}/{state.info.arch}
          </dd>
        </dl>
      )}

      {state.status === "error" && <p className="empty-note">{state.message}</p>}
    </section>
  );
}
