import type { PermissionsSnapshot } from "./permissions";
import type { ProviderConfig, RuntimeState } from "../types";

/**
 * Which step of first run the user is on.
 *
 * Derived from what is actually true rather than stored as a counter: someone
 * who revokes microphone permission halfway through, or quits and comes back,
 * should land on the step that is genuinely blocking them — not on step 4
 * because a number said so.
 */
export type Step = "permissions" | "speech-model" | "provider" | "detection" | "ready";

export interface Progress {
  /** Both capture permissions granted. */
  permissions: boolean;
  /** The speech model is loaded, so a recording will produce words. */
  speechModel: boolean;
  /** A summariser is reachable: a key stored, or a local runtime installed. */
  provider: boolean;
  /** The user has seen the detection explanation. */
  detectionSeen: boolean;
}

export const STEPS: { id: Step; title: string }[] = [
  { id: "permissions", title: "Let Oatmeal hear the meeting" },
  { id: "speech-model", title: "Download the speech model" },
  { id: "provider", title: "Choose who writes the summary" },
  { id: "detection", title: "How Oatmeal notices meetings" },
];

/**
 * Whether a provider is usable without further setup.
 *
 * A cloud provider needs a key; the local paths need something installed. An
 * onboarding that calls itself finished while generation would fail is worse
 * than one that asks a question.
 */
export function providerReady(
  config: ProviderConfig | null,
  hasKey: boolean,
  runtime: RuntimeState | null,
): boolean {
  if (!config) return false;
  if (config.kind === "bundled") {
    return runtime?.state === "ready" || runtime?.state === "running";
  }
  if (config.kind === "ollama" || config.kind === "lmstudio") {
    // Reachability is only knowable by trying, and blocking first run on a
    // round trip to a server the user may be about to start is worse than
    // letting them continue and finding out at generation time.
    return true;
  }
  return hasKey;
}

/** The first step that is not yet satisfied. */
export function currentStep(progress: Progress): Step {
  if (!progress.permissions) return "permissions";
  if (!progress.speechModel) return "speech-model";
  if (!progress.provider) return "provider";
  if (!progress.detectionSeen) return "detection";
  return "ready";
}

/** How far along, for the progress line. */
export function stepNumber(step: Step): number {
  const index = STEPS.findIndex((s) => s.id === step);
  return index === -1 ? STEPS.length : index + 1;
}

/**
 * Whether first run should be shown at all.
 *
 * Skipped once the user has dismissed it *or* recorded something — a returning
 * user with a meeting in the library has plainly got past setup, and putting a
 * wizard in front of them would be an insult.
 */
export function shouldShow(
  dismissed: boolean,
  meetingCount: number,
  progress: Progress,
): boolean {
  if (dismissed || meetingCount > 0) return false;
  return currentStep(progress) !== "ready";
}

/** Derives progress from what the app can actually see. */
export function readProgress(input: {
  permissions: PermissionsSnapshot | null;
  modelReady: boolean;
  config: ProviderConfig | null;
  hasKey: boolean;
  runtime: RuntimeState | null;
  detectionSeen: boolean;
}): Progress {
  return {
    permissions:
      input.permissions?.microphone === "granted" &&
      input.permissions?.screenRecording === "granted",
    speechModel: input.modelReady,
    provider: providerReady(input.config, input.hasKey, input.runtime),
    detectionSeen: input.detectionSeen,
  };
}
