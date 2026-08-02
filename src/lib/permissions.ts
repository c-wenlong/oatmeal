import type { PermissionState, PrivacyPane } from "../types";

export interface PermissionsSnapshot {
  microphone: PermissionState;
  screenRecording: PermissionState;
  needsRelaunch: boolean;
}

export interface CapabilityView {
  pane: PrivacyPane;
  title: string;
  /** Why Oatmeal needs it, in the user's terms. */
  reason: string;
  state: PermissionState;
  tone: "ok" | "pending" | "err";
  /** What the user must do next; null when nothing is required. */
  remedy: string | null;
  /** Whether prompting can still work — false once macOS has recorded a denial. */
  promptable: boolean;
}

function view(
  pane: PrivacyPane,
  title: string,
  reason: string,
  state: PermissionState,
): CapabilityView {
  switch (state) {
    case "granted":
      return {
        pane,
        title,
        reason,
        state,
        tone: "ok",
        remedy: null,
        promptable: false,
      };
    case "undetermined":
      return {
        pane,
        title,
        reason,
        state,
        tone: "pending",
        remedy: "Not asked yet — Oatmeal can prompt for this.",
        promptable: true,
      };
    case "denied":
      // macOS never re-shows a prompt once denied, so offering "ask again" here
      // would be a button that silently does nothing.
      return {
        pane,
        title,
        reason,
        state,
        tone: "err",
        remedy: `Denied. Enable it in System Settings, then relaunch.`,
        promptable: false,
      };
  }
}

export function capabilities(snapshot: PermissionsSnapshot): CapabilityView[] {
  return [
    view(
      "microphone",
      "Microphone",
      "Captures your voice. Without it, only the other side of a call is transcribed.",
      snapshot.microphone,
    ),
    view(
      "screen_recording",
      "Screen & System Audio Recording",
      "Captures what everyone else says. macOS has no audio-only permission — ScreenCaptureKit requires this one even though Oatmeal never records your screen.",
      snapshot.screenRecording,
    ),
  ];
}

/**
 * Mirrors `SidecarEvent::blocks_capture` in Rust.
 *
 * Recording without both capabilities produces a silent or half-empty
 * transcript, which is worse than refusing outright — so any gap blocks.
 */
export function blocksCapture(snapshot: PermissionsSnapshot): boolean {
  return (
    snapshot.needsRelaunch ||
    snapshot.microphone !== "granted" ||
    snapshot.screenRecording !== "granted"
  );
}

/** True when prompting could still change something. */
export function canPrompt(snapshot: PermissionsSnapshot): boolean {
  return (
    snapshot.microphone === "undetermined" ||
    snapshot.screenRecording === "undetermined"
  );
}

export function headline(snapshot: PermissionsSnapshot): string {
  if (snapshot.needsRelaunch) {
    // The nastiest state: the checkbox is on, both read granted, and capture
    // still yields nothing. Say the actual fix rather than "granted".
    return "Screen Recording was granted after Oatmeal started. Relaunch to pick it up.";
  }
  if (!blocksCapture(snapshot)) return "Ready to record.";
  if (canPrompt(snapshot)) return "Oatmeal needs permission before it can record.";
  return "Recording is blocked until both permissions are enabled.";
}
