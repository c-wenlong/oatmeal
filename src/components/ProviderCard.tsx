import { useCallback, useEffect, useState } from "react";
import {
  providerCurrent,
  providerSelect,
  providerSetKey,
  providerTest,
  providersList,
  runtimeState,
} from "../lib/tauri";
import type {
  ProviderConfig,
  ProviderInfo,
  ProviderKind,
  RuntimeState,
} from "../types";

/**
 * Which model writes the summaries.
 *
 * Local providers are listed first and need no key — that path is the default,
 * and the card says plainly whether a given choice keeps the transcript on the
 * machine.
 */
export function ProviderCard() {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [config, setConfig] = useState<ProviderConfig | null>(null);
  const [model, setModel] = useState("");
  const [keyInput, setKeyInput] = useState("");
  const [runtime, setRuntime] = useState<RuntimeState | null>(null);
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [list, current] = await Promise.all([providersList(), providerCurrent()]);
      setProviders(list);
      setConfig(current);
      setModel(current.model);
      setRuntime(await runtimeState());
    } catch (err: unknown) {
      setMessage(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const selected = providers.find((p) => p.kind === config?.kind) ?? null;

  async function guard(action: () => Promise<void>) {
    setMessage(null);
    try {
      await action();
    } catch (err: unknown) {
      setMessage(String(err));
    }
  }

  const choose = (kind: ProviderKind) =>
    guard(async () => {
      setResult(null);
      const next = await providerSelect(kind);
      setConfig(next);
      setModel(next.model);
    });

  const applyModel = () =>
    guard(async () => {
      if (!config) return;
      const next = await providerSelect(config.kind, model);
      setConfig(next);
    });

  const saveKey = () =>
    guard(async () => {
      if (!config) return;
      await providerSetKey(config.kind, keyInput);
      // Never held in component state beyond this call.
      setKeyInput("");
      await refresh();
    });

  const test = () =>
    guard(async () => {
      setTesting(true);
      setResult(null);
      try {
        setResult(await providerTest());
      } finally {
        setTesting(false);
      }
    });

  return (
    <section className="card">
      <div className="card-head">
        <h2>Summarisation model</h2>
        {selected && (
          <span className={selected.isLocal ? "pill pill--ok" : "pill pill--pending"}>
            {selected.isLocal ? "on device" : "sends to cloud"}
          </span>
        )}
      </div>
      <p className="card-note">
        Transcription is always on device. This is the only step that can leave your
        machine, and only if you pick a cloud provider.
      </p>

      <div className="row">
        {providers.map((provider) => (
          <button
            key={provider.kind}
            className={provider.kind === config?.kind ? "primary" : ""}
            onClick={() => choose(provider.kind)}
          >
            {provider.label}
          </button>
        ))}
      </div>

      {config && selected && (
        <div style={{ marginTop: 16 }}>
          <dl className="kv">
            <dt>Endpoint</dt>
            <dd>{config.baseUrl}</dd>
          </dl>

          <div className="row" style={{ marginTop: 10 }}>
            <input
              aria-label="Model"
              value={model}
              onChange={(e) => setModel(e.currentTarget.value)}
              placeholder="model name"
            />
            <button onClick={applyModel}>Set model</button>
          </div>

          {selected.requiresKey && (
            <div className="row" style={{ marginTop: 10 }}>
              <input
                aria-label="API key"
                type="password"
                value={keyInput}
                onChange={(e) => setKeyInput(e.currentTarget.value)}
                placeholder={
                  selected.hasKey
                    ? "key stored — enter a new one to replace"
                    : "paste your API key"
                }
              />
              <button onClick={saveKey}>{keyInput ? "Save key" : "Remove key"}</button>
              {selected.hasKey && <span className="pill pill--ok">key stored</span>}
            </div>
          )}

          {selected.kind === "bundled" && runtime && (
            <p className="empty-note">
              {runtime.state === "not_installed" &&
                "Not installed yet. Oatmeal downloads llama-server and a model on first use."}
              {runtime.state === "needs_model" &&
                "Runtime installed; no model downloaded yet."}
              {runtime.state === "ready" && "Ready to start."}
              {runtime.state === "running" && `Running (pid ${runtime.pid}).`}
            </p>
          )}

          <div className="row" style={{ marginTop: 12 }}>
            <button className="primary" onClick={test} disabled={testing}>
              {testing ? "Testing…" : "Test connection"}
            </button>
            {result && <span className="pill pill--ok">replied: {result}</span>}
          </div>
        </div>
      )}

      {message && <p className="empty-note">{message}</p>}
    </section>
  );
}
