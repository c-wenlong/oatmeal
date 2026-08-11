import { useCallback, useEffect, useState } from "react";
import {
  onDownloadProgress,
  providerCurrent,
  providerModelAvailable,
  providerPullModel,
  providerSelect,
  providerSetKey,
  providerTest,
  providersList,
  runtimeState,
} from "../lib/tauri";
import { ModelPicker } from "./ModelPicker";
import type {
  ModelAvailability,
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
/**
 * What to tell the user about the chosen local model, and whether a Download
 * button could possibly help.
 *
 * Three states, not two. "Ollama is not running" and "the model is not pulled"
 * are fixed in different places, and a Download button offered for the first
 * one cannot work — it would fail against the same unreachable server.
 */
export function modelAdvice(availability: ModelAvailability | null): {
  note: string | null;
  canPull: boolean;
} {
  if (!availability) return { note: null, canPull: false };
  switch (availability.state) {
    case "installed":
      return { note: null, canPull: false };
    case "missing":
      return {
        note: `${availability.model} is not installed. Oatmeal can download it, or pick a model you already have.`,
        canPull: true,
      };
    case "unreachable":
      // Naming the endpoint, because the usual cause is that Ollama is not
      // running and the second most usual is a base URL pointing elsewhere.
      return { note: availability.detail, canPull: false };
  }
}

/** Progress on the pull button, in whole megabytes. */
export function pullLabel(progress: { done: number; total: number | null }): string {
  const mb = (bytes: number) => Math.round(bytes / 1_048_576);
  // Ollama reports each layer as its own run of bytes, so a percentage of the
  // whole is not available and a fake one would run backwards.
  if (!progress.total) return "Downloading…";
  return `Downloading ${mb(progress.done)}/${mb(progress.total)} MB`;
}

export function ProviderCard() {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [config, setConfig] = useState<ProviderConfig | null>(null);
  const [model, setModel] = useState("");
  const [keyInput, setKeyInput] = useState("");
  const [runtime, setRuntime] = useState<RuntimeState | null>(null);
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [availability, setAvailability] = useState<ModelAvailability | null>(null);
  const [pulling, setPulling] = useState<{ done: number; total: number | null } | null>(
    null,
  );

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
    // Separately, and after: reaching Ollama can take a moment and a failure
    // here says nothing about the rest of the card.
    try {
      setAvailability(await providerModelAvailable());
    } catch {
      setAvailability(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  /* The pull streams progress on the same channel the bundled runtime uses, so
     there is one progress renderer rather than two. */
  useEffect(() => {
    const handle = onDownloadProgress((progress) => {
      setPulling(
        progress.done ? null : { done: progress.downloaded, total: progress.total },
      );
      if (progress.done) void refresh();
    });
    return () => {
      void handle.then((off) => off());
    };
  }, [refresh]);

  async function pull() {
    setPulling({ done: 0, total: null });
    try {
      await providerPullModel();
      await refresh();
    } catch (err: unknown) {
      setMessage(String(err));
    } finally {
      setPulling(null);
    }
  }

  const selected = providers.find((p) => p.kind === config?.kind) ?? null;
  const advice = modelAdvice(availability);

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

          {/* Only ever shown when there is something wrong: a card that
              announces "installed" on every render is one more thing to read
              on a page where nothing is happening. */}
          {advice.note && (
            <div className="row" style={{ marginTop: 10 }}>
              <p className="empty-note" style={{ flex: 1, margin: 0 }}>
                {advice.note}
              </p>
              {advice.canPull && (
                <button
                  className="primary"
                  onClick={() => void pull()}
                  disabled={!!pulling}
                >
                  {pulling ? pullLabel(pulling) : `Download ${config.model}`}
                </button>
              )}
            </div>
          )}

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
            <>
              <p className="empty-note">
                {runtime.state === "not_installed" &&
                  "Nothing installed yet — start with step 1 below."}
                {runtime.state === "needs_model" &&
                  "Server installed; pick a model below."}
                {runtime.state === "ready" && "Ready to start."}
                {runtime.state === "running" && `Running (pid ${runtime.pid}).`}
              </p>
              {runtime.state !== "running" && <ModelPicker />}
            </>
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
