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

/**
 * Whether a row should ask macOS, or send the user to System Settings.
 *
 * Screen Recording is the awkward one: CoreGraphics has no way to tell
 * "never asked" from "denied", so it never reports `undetermined` and its
 * `promptable` is always false once it is not granted. Refusing to prompt on
 * that basis would send a first-run user to System Settings for a permission a
 * single dialog would have granted.
 *
 * Asking anyway is harmless — when a denial is already recorded the call
 * returns without showing anything — so it is offered once. If the state has
 * not moved afterwards, the prompt did not appear and Settings is the only
 * remaining route.
 */
export function rowAction(
  capability: Pick<CapabilityView, "pane" | "promptable">,
  alreadyAsked: boolean,
): "prompt" | "settings" {
  if (capability.promptable) return "prompt";
  if (capability.pane === "screen_recording" && !alreadyAsked) return "prompt";
  return "settings";
}

/**
 * What flipping a permission switch should actually do.
 *
 * The switch reads as a switch, but macOS owns the state and gives an app no
 * way to write it directly. So each direction maps to the one thing that can
 * really happen:
 *
 * - **off → on** asks macOS, which shows the dialog while one can still
 *   appear, and otherwise opens the pane where the checkbox lives.
 * - **on → off** can only ever be Settings. No API revokes an app's own TCC
 *   grant, and a switch that silently refused to move would be worse than one
 *   that takes you where the move is possible.
 */
export function toggleAction(
  capability: Pick<CapabilityView, "pane" | "promptable" | "state">,
  alreadyAsked: boolean,
): "prompt" | "settings" {
  if (capability.state === "granted") return "settings";
  return rowAction(capability, alreadyAsked);
}
