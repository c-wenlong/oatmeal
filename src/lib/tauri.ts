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
  ModelOption,
  ModelStatus,
  DownloadProgress,
  Candidate,
  DetectionOutcome,
  DetectionSettings,
  DetectionRule,
  AppQuestion,
  Folder,
  SearchResponse,
  ChatReply,
  PrivacySnapshot,
  Retention,
  SweepReport,
  NotionDatabase,
  NotionSettings,
  ExportResult,
  GcalSettings,
  FlowOutcome,
  UpdateStatus,
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

/** The curated model list the bundled runtime can fetch. */
export function runtimeModels(): Promise<ModelOption[]> {
  return invoke<ModelOption[]>("runtime_models");
}

export function runtimeModelStatus(): Promise<[string, ModelStatus][]> {
  return invoke<[string, ModelStatus][]>("runtime_model_status");
}

/** Downloads llama-server. Progress arrives via `onDownloadProgress`. */
export function runtimeInstallServer(): Promise<void> {
  return invoke<void>("runtime_install_server");
}

/** Downloads a model. Gigabytes — always pair with progress and cancel. */
export function runtimeInstallModel(modelId: string): Promise<void> {
  return invoke<void>("runtime_install_model", { modelId });
}

/** Asks the running download to stop. Bytes already fetched are kept. */
export function runtimeCancelDownload(): Promise<void> {
  return invoke<void>("runtime_cancel_download");
}

export function onDownloadProgress(
  handler: (progress: DownloadProgress) => void,
): Promise<UnlistenFn> {
  return listen<DownloadProgress>("runtime://download", (e) => handler(e.payload));
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

// ----------------------------------------------------------------- detection

/** Answers an offer. Returns the new meeting id when the answer was "start". */
export function detectionRespond(
  candidateId: string,
  outcome: DetectionOutcome,
): Promise<string | null> {
  return invoke<string | null>("detection_respond", { candidateId, outcome });
}

/** Records "always" or "never" for an app nobody has ruled on yet. */
export function detectionAnswerApp(
  bundleId: string,
  appName: string | null,
  allow: boolean,
): Promise<void> {
  return invoke<void>("detection_answer_app", { bundleId, appName, allow });
}

/** The app awaiting a one-time answer, if any. */
export function detectionPendingQuestion(): Promise<AppQuestion | null> {
  return invoke<AppQuestion | null>("detection_pending_question");
}

export function detectionCandidates(): Promise<Candidate[]> {
  return invoke<Candidate[]>("detection_candidates");
}

export function detectionSettings(): Promise<DetectionSettings> {
  return invoke<DetectionSettings>("detection_settings");
}

export function detectionSetSettings(settings: DetectionSettings): Promise<void> {
  return invoke<void>("detection_set_settings", { settings });
}

export function detectionRulesList(): Promise<DetectionRule[]> {
  return invoke<DetectionRule[]>("detection_rules_list");
}

export function detectionRuleClear(bundleId: string): Promise<void> {
  return invoke<void>("detection_rule_clear", { bundleId });
}

/** The shipped allowlist, as (bundleId, name) pairs. */
export function detectionBuiltinApps(): Promise<[string, string][]> {
  return invoke<[string, string][]>("detection_builtin_apps");
}

export function onCandidates(
  handler: (candidates: Candidate[]) => void,
): Promise<UnlistenFn> {
  return listen<Candidate[]>("detect://candidates", (e) => handler(e.payload));
}

export function onAppQuestion(
  handler: (question: AppQuestion) => void,
): Promise<UnlistenFn> {
  return listen<AppQuestion>("detect://ask", (e) => handler(e.payload));
}

// ------------------------------------------------------------ folders + search

export function foldersList(): Promise<Folder[]> {
  return invoke<Folder[]>("folders_list");
}

export function folderCreate(name: string, parentId?: string | null): Promise<string> {
  return invoke<string>("folder_create", { name, parentId: parentId ?? null });
}

export function folderRename(folderId: string, name: string): Promise<void> {
  return invoke<void>("folder_rename", { folderId, name });
}

/** Deletes a folder. Its meetings survive and become unfiled. */
export function folderDelete(folderId: string): Promise<void> {
  return invoke<void>("folder_delete", { folderId });
}

/** Meetings in a folder, or unfiled ones when `folderId` is null. */
export function folderMeetings(folderId: string | null): Promise<MeetingSummary[]> {
  return invoke<MeetingSummary[]>("folder_meetings", { folderId });
}

export function meetingSetFolder(
  meetingId: string,
  folderId: string | null,
): Promise<void> {
  return invoke<void>("meeting_set_folder", { meetingId, folderId });
}

export function searchTranscripts(
  query: string,
  folderId: string | null,
): Promise<SearchResponse> {
  return invoke<SearchResponse>("search_transcripts", { query, folderId });
}

/** Asks a question over one meeting or a whole folder. */
export function chatAsk(
  question: string,
  meetingId: string | null,
  folderId: string | null,
): Promise<ChatReply> {
  return invoke<ChatReply>("chat_ask", { question, meetingId, folderId });
}

// -------------------------------------------------------------------- privacy

export function privacySnapshot(): Promise<PrivacySnapshot> {
  return invoke<PrivacySnapshot>("privacy_snapshot");
}

export function privacySetRetention(retention: Retention): Promise<void> {
  return invoke<void>("privacy_set_retention", { retention });
}

/** Deletes every audio file. Transcripts survive. */
export function privacyPurgeAudio(): Promise<SweepReport> {
  return invoke<SweepReport>("privacy_purge_audio");
}

// --------------------------------------------------------------------- notion

export function notionSettings(): Promise<NotionSettings> {
  return invoke<NotionSettings>("notion_settings");
}

/** Stores the integration token, or clears it with an empty string. */
export function notionSetToken(token: string): Promise<void> {
  return invoke<void>("notion_set_token", { token });
}

export function notionSetOptions(
  databaseId: string | null,
  includeTranscript: boolean,
  autoExport: boolean,
): Promise<void> {
  return invoke<void>("notion_set_options", {
    databaseId,
    includeTranscript,
    autoExport,
  });
}

export function notionDatabases(): Promise<NotionDatabase[]> {
  return invoke<NotionDatabase[]>("notion_databases");
}

/** Creates the meeting's page, or updates the one it already has. */
export function notionExport(meetingId: string): Promise<ExportResult> {
  return invoke<ExportResult>("notion_export", { meetingId });
}

// ------------------------------------------------------------ google calendar

export function gcalSettings(): Promise<GcalSettings> {
  return invoke<GcalSettings>("gcal_settings");
}

/** The user's own OAuth client id. Not a secret for an installed app. */
export function gcalSetClientId(clientId: string): Promise<void> {
  return invoke<void>("gcal_set_client_id", { clientId });
}

export function gcalSetEnabled(enabled: boolean): Promise<void> {
  return invoke<void>("gcal_set_enabled", { enabled });
}

/** Opens the browser and waits for the redirect. Resolves when the flow ends. */
export function gcalConnect(): Promise<FlowOutcome> {
  return invoke<FlowOutcome>("gcal_connect");
}

export function gcalDisconnect(): Promise<void> {
  return invoke<void>("gcal_disconnect");
}

// ------------------------------------------------------------------- updates

export function updateCheck(): Promise<UpdateStatus> {
  return invoke<UpdateStatus>("update_check");
}

/** Downloads, swaps the bundle, and restarts. Does not return on success. */
export function updateInstall(): Promise<void> {
  return invoke<void>("update_install");
}

export function updateSkip(version: string): Promise<void> {
  return invoke<void>("update_skip", { version });
}

/**
 * A meeting with nothing recorded in it — somewhere to type.
 *
 * Not `meetingStart`, which begins a capture and needs a running sidecar.
 */
export function meetingCreate(title?: string): Promise<string> {
  return invoke<string>("meeting_create", { title });
}
