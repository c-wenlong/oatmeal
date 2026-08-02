import { useCallback, useEffect, useState } from "react";
import {
  onDownloadProgress,
  runtimeCancelDownload,
  runtimeInstallModel,
  runtimeInstallServer,
  runtimeModelStatus,
  runtimeModels,
  runtimeState,
} from "../lib/tauri";
import type {
  DownloadProgress,
  ModelOption,
  ModelStatus,
  RuntimeState,
} from "../types";

/** Bytes as something a person can read. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/**
 * What a download button should say, given what is already on disk.
 *
 * Split out because "Resume" versus "Download" is the difference between the
 * user believing they are about to re-fetch four gigabytes and knowing they are
 * not.
 */
export function downloadLabel(status: ModelStatus | undefined): string {
  switch (status?.status) {
    case "installed":
      return "Installed";
    case "partial":
      return `Resume from ${formatBytes(status.bytes)}`;
    default:
      return "Download";
  }
}

/**
 * The guided download G13 asks for: fetch the server, then a model, without a
 * terminal.
 *
 * This is what makes the offline path real rather than aspirational — a user
 * with no API key and no Ollama can get to a working summary from here.
 */
export function ModelPicker() {
  const [state, setState] = useState<RuntimeState | null>(null);
  const [models, setModels] = useState<ModelOption[]>([]);
  const [status, setStatus] = useState<Record<string, ModelStatus>>({});
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setState(await runtimeState());
      setModels(await runtimeModels());
      setStatus(Object.fromEntries(await runtimeModelStatus()));
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const handle = onDownloadProgress(setProgress);
    return () => {
      void handle.then((off) => off?.());
    };
  }, []);

  async function run(id: string, action: () => Promise<void>) {
    setBusy(id);
    setError(null);
    setProgress(null);
    try {
      await action();
    } catch (err) {
      // A cancel arrives here too. It is not a failure, and reporting it as one
      // would make stopping a download feel like something went wrong.
      const text = String(err);
      setError(text.includes("cancelled") ? null : text);
    } finally {
      setBusy(null);
      setProgress(null);
      await refresh();
    }
  }

  const serverInstalled = state !== null && state.state !== "not_installed";
  const fraction =
    progress && progress.total ? progress.downloaded / progress.total : null;

  return (
    <div className="picker" data-testid="model-picker">
      <p className="card-note">
        Runs entirely on this machine — no API key, no account, nothing leaves the
        laptop. Two downloads: the server, then a model.
      </p>

      <ol className="picker-steps">
        <li
          className={serverInstalled ? "picker-step picker-step--done" : "picker-step"}
        >
          <div className="picker-step-head">
            <span>1 · Inference server</span>
            {serverInstalled ? (
              <span className="pill pill--ok">installed</span>
            ) : (
              <button
                onClick={() => void run("server", runtimeInstallServer)}
                disabled={busy !== null}
              >
                {busy === "server" ? "Downloading…" : "Download (~11 MB)"}
              </button>
            )}
          </div>
          <p className="empty-note">llama.cpp, about 11 MB.</p>
        </li>

        <li className="picker-step">
          <div className="picker-step-head">
            <span>2 · Model</span>
          </div>
          {!serverInstalled && <p className="empty-note">Install the server first.</p>}
          {serverInstalled &&
            models.map((model) => {
              const installed = status[model.id]?.status === "installed";
              return (
                <div className="picker-model" key={model.id}>
                  <div className="picker-model-head">
                    <strong>{model.name}</strong>
                    <span className="empty-note">{formatBytes(model.approxBytes)}</span>
                    <button
                      onClick={() =>
                        void run(model.id, () => runtimeInstallModel(model.id))
                      }
                      disabled={busy !== null || installed}
                    >
                      {busy === model.id
                        ? "Downloading…"
                        : downloadLabel(status[model.id])}
                    </button>
                  </div>
                  <p className="empty-note">{model.note}</p>
                </div>
              );
            })}
        </li>
      </ol>

      {busy !== null && (
        <div className="picker-progress">
          <div className="picker-bar">
            <div
              className="picker-bar-fill"
              style={{ width: `${Math.round((fraction ?? 0) * 100)}%` }}
              data-testid="progress-fill"
            />
          </div>
          <div className="row">
            <span className="empty-note">
              {progress
                ? `${formatBytes(progress.downloaded)}${
                    progress.total ? ` of ${formatBytes(progress.total)}` : ""
                  }`
                : "Starting…"}
            </span>
            <button onClick={() => void runtimeCancelDownload()}>Cancel</button>
          </div>
          <p className="empty-note">
            Cancelling keeps what has downloaded so far — you can resume later.
          </p>
        </div>
      )}

      {error && <p className="empty-note">{error}</p>}
    </div>
  );
}
