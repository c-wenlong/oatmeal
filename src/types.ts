/** Mirrors `HealthInfo` in src-tauri/src/lib.rs. */
export interface HealthInfo {
  appVersion: string;
  buildProfile: string;
  arch: string;
  os: string;
}

/** Mirrors `DbStats` in src-tauri/src/db/repo.rs. */
export interface DbStats {
  schemaVersion: number;
  meetings: number;
  utterances: number;
  noteBlocks: number;
  panels: number;
  embeddings: number;
}

/** Mirrors `AudioSource` in src-tauri/src/sidecar/protocol.rs. */
export type AudioSource = "mic" | "system";

/** Mirrors `PermissionState` in src-tauri/src/sidecar/protocol.rs. */
export type PermissionState = "granted" | "denied" | "undetermined";

/** Mirrors `ModelState` in src-tauri/src/sidecar/protocol.rs. */
export type ModelState = "downloading" | "loading" | "ready" | "failed";

/** Mirrors `PrivacyPane` in src-tauri/src/lib.rs. */
export type PrivacyPane = "microphone" | "screen_recording";

/** Mirrors `SidecarEvent` in src-tauri/src/sidecar/protocol.rs. */
export type SidecarEvent =
  | { ev: "ready"; version: string; protocol: number }
  | { ev: "partial"; source: AudioSource; text: string; t0: number; t1: number }
  | {
      ev: "final";
      source: AudioSource;
      text: string;
      t0: number;
      t1: number;
      conf: number | null;
    }
  | { ev: "level"; mic: number; system: number }
  | { ev: "stopped"; audio_path: string | null; duration_ms: number }
  | { ev: "error"; message: string; fatal: boolean }
  | { ev: "pong" }
  | {
      ev: "permissions";
      microphone: PermissionState;
      screen_recording: PermissionState;
      needs_relaunch: boolean;
    }
  | {
      ev: "model";
      name: string;
      state: ModelState;
      progress: number | null;
      message: string | null;
    };

/** Mirrors `SupervisorEvent` in src-tauri/src/sidecar/supervisor.rs. */
export type SupervisorEvent =
  | { kind: "spawned"; pid: number; attempt: number }
  | { kind: "event"; event: SidecarEvent }
  | { kind: "garbled"; line: string; error: string }
  | { kind: "exited"; code: number | null; restarting_in_ms: number | null }
  | { kind: "gave_up"; reason: string };

/** Mirrors `SidecarCommand` in src-tauri/src/sidecar/protocol.rs. */
export type SidecarCommand =
  | { cmd: "start"; meeting_id: string; sources: AudioSource[] }
  | { cmd: "stop" }
  | { cmd: "ping" }
  | { cmd: "arm" }
  | { cmd: "disarm" }
  | { cmd: "permissions"; request: boolean };

/** Mirrors `DbSelftest` in src-tauri/src/lib.rs. */
export interface DbSelftest {
  schemaVersion: number;
  dbPath: string;
  stats: DbStats;
  /** Text the FTS index returned for a stemmed query; null means stemming failed. */
  ftsHit: string | null;
  /** Owner id of the nearest vector; null means sqlite-vec returned nothing. */
  vectorHit: string | null;
}

/** Mirrors `MeetingSummary` in src-tauri/src/db/repo.rs. */
export interface MeetingSummary {
  id: string;
  title: string | null;
  startedAt: number;
  endedAt: number | null;
  status: string;
  audioPath: string | null;
  utteranceCount: number;
}

/** Mirrors `Utterance` in src-tauri/src/db/repo.rs. */
export interface Utterance {
  id: number;
  seq: number;
  source: AudioSource;
  text: string;
  startMs: number;
  endMs: number;
  confidence: number | null;
}

/** Mirrors `NoteBlock` in src-tauri/src/db/repo.rs. */
export interface NoteBlock {
  /** Editor-assigned and stable for the life of the block. */
  blockId: string;
  seq: number;
  text: string;
  /** Milliseconds from meeting start to the first keystroke. Never rewritten. */
  firstTypedAtMs: number | null;
  lastEditedAtMs: number | null;
}

/** Mirrors `MeetingState` in src-tauri/src/meeting.rs. */
export type MeetingState =
  | { state: "idle" }
  | { state: "armed" }
  | { state: "recording"; meeting_id: string }
  /** Stop requested; the sidecar has not finalised the file yet. */
  | { state: "processing"; meeting_id: string };

/** Mirrors `ProviderKind` in src-tauri/src/llm/provider.rs. */
export type ProviderKind =
  "anthropic" | "openai" | "openrouter" | "ollama" | "lmstudio" | "bundled";

/** Mirrors `ProviderInfo` in src-tauri/src/lib.rs. */
export interface ProviderInfo {
  kind: ProviderKind;
  label: string;
  defaultBaseUrl: string;
  defaultModel: string;
  requiresKey: boolean;
  isLocal: boolean;
  /** Whether a key is stored. Never the key itself. */
  hasKey: boolean;
}

/** Mirrors `ProviderConfig` in src-tauri/src/llm/provider.rs. */
export interface ProviderConfig {
  id: string;
  kind: ProviderKind;
  baseUrl: string;
  model: string;
  keychainRef: string | null;
}

/** Mirrors `Template` in src-tauri/src/panel/prompt.rs. */
export interface Template {
  id: string;
  name: string;
  prompt: string;
  isBuiltin: boolean;
}

/** Mirrors `Bullet` in src-tauri/src/panel/content.rs. */
export interface Bullet {
  text: string;
  /** Transcript line ids. Empty means the claim could not be traced. */
  sourceUtterances: number[];
  fromNote: string | null;
}

export interface PanelSection {
  heading: string;
  bullets: Bullet[];
}

export interface PanelContent {
  sections: PanelSection[];
}

/** Mirrors `Panel` in src-tauri/src/db/repo.rs. */
export interface Panel {
  id: string;
  templateId: string | null;
  contentJson: string;
  provider: string | null;
  model: string | null;
  generatedAt: number;
}

/** Mirrors `RuntimeState` in src-tauri/src/llm/bundled.rs. */
export type RuntimeState =
  | { state: "not_installed" }
  | { state: "needs_model" }
  | { state: "ready" }
  | { state: "running"; pid: number };

/** Mirrors `ModelOption` in src-tauri/src/llm/bundled.rs. */
export interface ModelOption {
  id: string;
  name: string;
  url: string;
  filename: string;
  approxBytes: number;
  note: string;
}

/** Mirrors `ModelStatus` in src-tauri/src/llm/bundled.rs. */
export type ModelStatus =
  { status: "absent" } | { status: "partial"; bytes: number } | { status: "installed" };

/** Mirrors `DownloadProgress` in src-tauri/src/llm/download.rs. */
export interface DownloadProgress {
  id: string;
  downloaded: number;
  total: number | null;
  done: boolean;
}

/** Mirrors `LinkMethod` in src-tauri/src/link/mod.rs. */
export type LinkMethod = "temporal" | "semantic" | "llm";

/** Mirrors `StoredLink` in src-tauri/src/db/repo.rs. */
export interface StoredLink {
  noteBlockId: string;
  utteranceId: number;
  method: LinkMethod;
  score: number;
}

/** Mirrors `LinkParams` in src-tauri/src/link/mod.rs. */
export interface LinkParams {
  lookBackMs: number;
  lookAheadMs: number;
  alpha: number;
  beta: number;
  globalMargin: number;
  minScore: number;
  maxPerNote: number;
}

/** Mirrors `IndexReport` in src-tauri/src/link/pipeline.rs. */
export interface IndexReport {
  embedded: number;
  links: number;
  /** Set when no embedder was reachable and linking fell back to timestamps. */
  degraded: string | null;
}
