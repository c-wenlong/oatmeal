import { describe, expect, it } from "vitest";
import {
  currentStep,
  providerReady,
  readProgress,
  shouldShow,
  stepNumber,
  type Progress,
} from "./onboarding";
import type { ProviderConfig, ProviderKind, RuntimeState } from "../types";

function progress(over: Partial<Progress> = {}): Progress {
  return {
    permissions: true,
    speechModel: true,
    provider: true,
    detectionSeen: true,
    ...over,
  };
}

function config(kind: ProviderKind): ProviderConfig {
  return {
    id: kind,
    kind,
    baseUrl: "http://localhost",
    model: "m",
    keychainRef: null,
  };
}

describe("providerReady", () => {
  it("needs a key for a cloud provider", () => {
    expect(providerReady(config("anthropic"), false, null)).toBe(false);
    expect(providerReady(config("anthropic"), true, null)).toBe(true);
  });

  it("does not block on reachability for a local server", () => {
    // Whether Ollama is running is only knowable by trying, and stalling first
    // run on a round trip to a server the user is about to start is worse than
    // finding out at generation time.
    expect(providerReady(config("ollama"), false, null)).toBe(true);
    expect(providerReady(config("lmstudio"), false, null)).toBe(true);
  });

  it("needs the bundled runtime actually installed", () => {
    // The one local path where "ready" is knowable, and claiming ready with
    // nothing downloaded would fail at the first summary.
    const notInstalled: RuntimeState = { state: "not_installed" };
    const needsModel: RuntimeState = { state: "needs_model" };
    const ready: RuntimeState = { state: "ready" };

    expect(providerReady(config("bundled"), false, notInstalled)).toBe(false);
    expect(providerReady(config("bundled"), false, needsModel)).toBe(false);
    expect(providerReady(config("bundled"), false, ready)).toBe(true);
    expect(providerReady(config("bundled"), false, { state: "running", pid: 1 })).toBe(
      true,
    );
  });

  it("is not ready with no provider chosen", () => {
    expect(providerReady(null, true, null)).toBe(false);
  });
});

describe("currentStep", () => {
  it("stops at the first thing that is missing", () => {
    expect(currentStep(progress({ permissions: false }))).toBe("permissions");
    expect(currentStep(progress({ speechModel: false }))).toBe("speech-model");
    expect(currentStep(progress({ provider: false }))).toBe("provider");
    expect(currentStep(progress({ detectionSeen: false }))).toBe("detection");
  });

  it("reports ready when everything is done", () => {
    expect(currentStep(progress())).toBe("ready");
  });

  it("goes back a step when something is revoked", () => {
    // Derived from what is true, not from a stored counter: revoking the
    // microphone must put the user back on the step that blocks them.
    expect(currentStep(progress({ permissions: false, detectionSeen: true }))).toBe(
      "permissions",
    );
  });
});

describe("stepNumber", () => {
  it("counts from one", () => {
    expect(stepNumber("permissions")).toBe(1);
    expect(stepNumber("detection")).toBe(4);
  });

  it("reports past the end when ready", () => {
    expect(stepNumber("ready")).toBe(4);
  });
});

describe("shouldShow", () => {
  it("shows on a fresh machine", () => {
    expect(shouldShow(false, 0, progress({ permissions: false }))).toBe(true);
  });

  it("does not show once dismissed", () => {
    expect(shouldShow(true, 0, progress({ permissions: false }))).toBe(false);
  });

  it("does not show to someone who has already recorded", () => {
    // A returning user with meetings in the library has plainly got past setup;
    // putting a wizard in front of them would be an insult.
    expect(shouldShow(false, 3, progress({ permissions: false }))).toBe(false);
  });

  it("does not show when there is nothing left to do", () => {
    expect(shouldShow(false, 0, progress())).toBe(false);
  });
});

describe("readProgress", () => {
  it("requires both capture permissions", () => {
    // One without the other still cannot record a meeting.
    const half = readProgress({
      permissions: {
        microphone: "granted",
        screenRecording: "denied",
        needsRelaunch: false,
      },
      modelReady: true,
      config: config("ollama"),
      hasKey: false,
      runtime: null,
      detectionSeen: true,
    });
    expect(half.permissions).toBe(false);
  });

  it("is satisfied when both are granted", () => {
    const both = readProgress({
      permissions: {
        microphone: "granted",
        screenRecording: "granted",
        needsRelaunch: false,
      },
      modelReady: true,
      config: config("ollama"),
      hasKey: false,
      runtime: null,
      detectionSeen: true,
    });
    expect(both.permissions).toBe(true);
    expect(currentStep(both)).toBe("ready");
  });

  it("treats unknown permissions as not granted", () => {
    const none = readProgress({
      permissions: null,
      modelReady: false,
      config: null,
      hasKey: false,
      runtime: null,
      detectionSeen: false,
    });
    expect(none.permissions).toBe(false);
    expect(currentStep(none)).toBe("permissions");
  });
});
