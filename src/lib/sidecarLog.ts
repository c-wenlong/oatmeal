import type { SidecarEvent, SupervisorEvent } from "../types";

export interface LogLine {
  /** Short label shown in the gutter. */
  tag: string;
  /** Drives the tag colour. */
  tone: "mic" | "system" | "meta" | "error";
  text: string;
  /** Partials are in-flight text; rendered muted because a `final` supersedes them. */
  partial: boolean;
}

/**
 * Turns a supervisor event into one displayable line.
 *
 * Pure and separate from the component so the formatting of every event variant
 * can be tested without rendering anything — and so an unhandled variant is a
 * type error rather than a blank row.
 */
export function formatSupervisorEvent(event: SupervisorEvent): LogLine {
  switch (event.kind) {
    case "spawned":
      return {
        tag: "spawn",
        tone: "meta",
        text: `sidecar started (pid ${event.pid}, attempt ${event.attempt})`,
        partial: false,
      };

    case "exited":
      return {
        tag: "exit",
        tone: "error",
        text:
          `sidecar exited${event.code === null ? "" : ` with code ${event.code}`}` +
          (event.restarting_in_ms === null
            ? " — not restarting"
            : ` — restarting in ${event.restarting_in_ms}ms`),
        partial: false,
      };

    case "gave_up":
      return { tag: "fatal", tone: "error", text: event.reason, partial: false };

    case "garbled":
      return {
        tag: "garbled",
        tone: "error",
        text: `${event.error} — ${event.line}`,
        partial: false,
      };

    case "event":
      return formatSidecarEvent(event.event);
  }
}

function formatSidecarEvent(event: SidecarEvent): LogLine {
  switch (event.ev) {
    case "ready":
      return {
        tag: "ready",
        tone: "meta",
        text: `handshake ok — v${event.version}, protocol ${event.protocol}`,
        partial: false,
      };

    case "partial":
      return {
        tag: event.source,
        tone: event.source,
        text: event.text,
        partial: true,
      };

    case "final":
      return {
        tag: event.source,
        tone: event.source,
        text:
          event.conf === null
            ? event.text
            : `${event.text}  (${Math.round(event.conf * 100)}%)`,
        partial: false,
      };

    case "level":
      return {
        tag: "level",
        tone: "meta",
        text: `mic ${event.mic.toFixed(2)} · system ${event.system.toFixed(2)}`,
        partial: true,
      };

    case "stopped":
      return {
        tag: "stop",
        tone: "meta",
        text: `session ended after ${(event.duration_ms / 1000).toFixed(1)}s`,
        partial: false,
      };

    case "error":
      return {
        tag: event.fatal ? "fatal" : "error",
        tone: "error",
        text: event.message,
        partial: false,
      };

    case "pong":
      return { tag: "pong", tone: "meta", text: "sidecar answered", partial: false };

    case "permissions": {
      const blocked =
        event.needs_relaunch ||
        event.microphone !== "granted" ||
        event.screen_recording !== "granted";
      return {
        tag: "perms",
        tone: blocked ? "error" : "meta",
        text:
          `mic ${event.microphone} · screen ${event.screen_recording}` +
          (event.needs_relaunch ? " · relaunch required" : ""),
        partial: false,
      };
    }

    case "model":
      return {
        tag: "model",
        tone: event.state === "failed" ? "error" : "meta",
        text:
          `${event.name}: ${event.state}` +
          (event.progress === null ? "" : ` ${Math.round(event.progress * 100)}%`) +
          (event.message === null ? "" : ` — ${event.message}`),
        // Download progress is transient; render it muted like a partial so it
        // doesn't read as a settled transcript line.
        partial: event.state === "downloading",
      };
  }
}

/** Timestamp for the log gutter, relative to when the card started listening. */
export function elapsedLabel(startedAt: number, now: number): string {
  const seconds = Math.max(0, (now - startedAt) / 1000);
  return seconds.toFixed(2).padStart(6, "0");
}
