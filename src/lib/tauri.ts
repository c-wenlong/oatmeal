import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DbSelftest,
  HealthInfo,
  MeetingState,
  MeetingSummary,
  PrivacyPane,
  SidecarCommand,
  NoteBlock,
  Panel,
  ProviderConfig,
  ProviderInfo,
  ProviderKind,
  RuntimeState,
  Template,
  SupervisorEvent,
  Utterance,
  StoredLink,
  LinkParams,
  IndexReport,
} from "../types";
import type { PermissionsSnapshot } from "./permissions";

/**
 * Thin wrapper over Tauri's `invoke`.
 *
 * Everything the frontend asks of the Rust core goes through this module, so
 * tests mock exactly one boundary instead of stubbing `invoke` at every call
 * site.
 */
export function healthCheck(): Promise<HealthInfo> {
  return invoke<HealthInfo>("health_check");
}

export function dbSelftest(): Promise<DbSelftest> {
  return invoke<DbSelftest>("db_selftest");
}

/** Resolves to the path of the sidecar binary that was spawned. */
export function sidecarStart(): Promise<string> {
  return invoke<string>("sidecar_start");
}

export function sidecarStop(): Promise<void> {
  return invoke<void>("sidecar_stop");
}

export function sidecarSend(command: SidecarCommand): Promise<void> {
  return invoke<void>("sidecar_send", { command });
}

export function sidecarSimulateCrash(): Promise<void> {
  return invoke<void>("sidecar_simulate_crash");
}

export function openPrivacySettings(pane: PrivacyPane): Promise<void> {
  return invoke<void>("open_privacy_settings", { pane });
}

/**
 * Last permission snapshot the Rust core saw, or null if the sidecar has never
 * reported. Permissions arrive as a one-shot event, so anything mounting after
 * it has no other way to learn the answer.
 */
export function permissionsSnapshot(): Promise<PermissionsSnapshot | null> {
  return invoke<PermissionsSnapshot | null>("permissions_snapshot");
}

/** Must match `SIDECAR_EVENT` in src-tauri/src/lib.rs. */
export const SIDECAR_EVENT = "sidecar://event";

export function onSidecarEvent(
  handler: (event: SupervisorEvent) => void,
): Promise<UnlistenFn> {
  return listen<SupervisorEvent>(SIDECAR_EVENT, (e) => handler(e.payload));
}

/** Creates the meeting row and starts recording into it. Returns its id. */
export function meetingStart(title?: string): Promise<string> {
  return invoke<string>("meeting_start", { title: title ?? null });
}

export function meetingStop(): Promise<void> {
  return invoke<void>("meeting_stop");
}

export function meetingActive(): Promise<string | null> {
  return invoke<string | null>("meeting_active");
}

export function meetingsList(): Promise<MeetingSummary[]> {
  return invoke<MeetingSummary[]>("meetings_list");
}

export function meetingTranscript(meetingId: string): Promise<Utterance[]> {
  return invoke<Utterance[]>("meeting_transcript", { meetingId });
}

export function notesSave(meetingId: string, blocks: NoteBlock[]): Promise<void> {
  return invoke<void>("notes_save", { meetingId, blocks });
}

export function notesLoad(meetingId: string): Promise<NoteBlock[]> {
  return invoke<NoteBlock[]>("notes_load", { meetingId });
}

export function meetingState(): Promise<MeetingState> {
  return invoke<MeetingState>("meeting_state");
}

export function meetingRename(meetingId: string, title: string): Promise<void> {
  return invoke<void>("meeting_rename", { meetingId, title });
}

export function meetingDelete(meetingId: string): Promise<void> {
  return invoke<void>("meeting_delete", { meetingId });
}

/** Must match `MEETING_STATE_EVENT` in src-tauri/src/lib.rs. */
export const MEETING_STATE_EVENT = "meeting://state";

/**
 * Subscribes to lifecycle changes. Rust owns the state — a meeting can be
 * started by something other than the record button (calendar detection, the
 * dev harness), and polling on mount alone would show "idle" during a live
 * recording.
 */
export function onMeetingState(
  handler: (state: MeetingState) => void,
): Promise<UnlistenFn> {
  return listen<MeetingState>(MEETING_STATE_EVENT, (e) => handler(e.payload));
}

export function providersList(): Promise<ProviderInfo[]> {
  return invoke<ProviderInfo[]>("providers_list");
}

export function providerCurrent(): Promise<ProviderConfig> {
  return invoke<ProviderConfig>("provider_current");
}

export function providerSelect(
  kind: ProviderKind,
  model?: string,
  baseUrl?: string,
): Promise<ProviderConfig> {
  return invoke<ProviderConfig>("provider_select", {
    kind,
    model: model ?? null,
    baseUrl: baseUrl ?? null,
  });
}

/** Stores a key in the Keychain. It is never read back into the frontend. */
export function providerSetKey(kind: ProviderKind, key: string): Promise<void> {
  return invoke<void>("provider_set_key", { kind, key });
}

export function providerTest(): Promise<string> {
  return invoke<string>("provider_test");
}

export function templatesList(): Promise<Template[]> {
  return invoke<Template[]>("templates_list");
}

export function panelsList(meetingId: string): Promise<Panel[]> {
  return invoke<Panel[]>("panels_list", { meetingId });
}

export function panelGenerate(meetingId: string, templateId: string): Promise<Panel> {
  return invoke<Panel>("panel_generate", { meetingId, templateId });
}

export function panelDelete(panelId: string): Promise<void> {
  return invoke<void>("panel_delete", { panelId });
}

export function runtimeState(): Promise<RuntimeState> {
  return invoke<RuntimeState>("runtime_state");
}

export function runtimeStart(): Promise<number> {
  return invoke<number>("runtime_start");
}

export function runtimeStop(): Promise<void> {
  return invoke<void>("runtime_stop");
}

// ------------------------------------------------------------------- linking

/** Embeds and re-links a meeting. Slow on a long transcript — it embeds. */
export function meetingIndex(meetingId: string): Promise<IndexReport> {
  return invoke<IndexReport>("meeting_index", { meetingId });
}

export function meetingLinks(meetingId: string): Promise<StoredLink[]> {
  return invoke<StoredLink[]>("meeting_links", { meetingId });
}

export function linkParamsGet(): Promise<LinkParams> {
  return invoke<LinkParams>("link_params_get");
}

/** Stores new weights. Does not re-link; call `meetingIndex` for that. */
export function linkParamsSet(params: LinkParams): Promise<void> {
  return invoke<void>("link_params_set", { params });
}

/** Fires when a background indexing pass finishes and links have changed. */
export function onMeetingIndexed(
  handler: (report: IndexReport) => void,
): Promise<UnlistenFn> {
  return listen<IndexReport>("meeting://indexed", (event) => handler(event.payload));
}
