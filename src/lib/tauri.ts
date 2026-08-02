import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DbSelftest,
  HealthInfo,
  MeetingSummary,
  PrivacyPane,
  SidecarCommand,
  SupervisorEvent,
  Utterance,
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
