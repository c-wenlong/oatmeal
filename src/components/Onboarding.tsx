import { useCallback, useEffect, useState } from "react";
import {
  meetingsList,
  onSidecarEvent,
  openPrivacySettings,
  permissionsSnapshot,
  providerCurrent,
  providersList,
  runtimeState,
  sidecarSend,
  sidecarStart,
} from "../lib/tauri";
import {
  STEPS,
  currentStep,
  readProgress,
  shouldShow,
  stepNumber,
  type Progress,
} from "../lib/onboarding";
import { ModelPicker } from "./ModelPicker";
import type { PermissionsSnapshot } from "../lib/permissions";
import type { ProviderConfig, RuntimeState } from "../types";

/** Where the dismissal is remembered. */
export const DISMISSED_KEY = "oatmeal.onboarding.dismissed";
export const DETECTION_SEEN_KEY = "oatmeal.onboarding.detection";

/**
 * First run.
 *
 * The goal is a first successful recording *without visiting a settings
 * screen*, so every step is actionable in place: the permission prompt, the
 * model download, and the provider choice all happen here rather than sending
 * the user somewhere and hoping they come back.
 *
 * Progress is derived from what is true, not counted. Someone who revokes a
 * permission mid-flow lands back on the step that blocks them.
 */
export function Onboarding() {
  const [permissions, setPermissions] = useState<PermissionsSnapshot | null>(null);
  const [modelReady, setModelReady] = useState(false);
  const [config, setConfig] = useState<ProviderConfig | null>(null);
  const [hasKey, setHasKey] = useState(false);
  const [runtime, setRuntime] = useState<RuntimeState | null>(null);
  const [meetings, setMeetings] = useState(0);
  /**
   * Whether the first look has finished.
   *
   * Without this, `meetings: 0` means both "no meetings yet" and "have not
   * checked". The setup card would flash on every return to the library:
   * mount, decide setup is needed because nothing is known, fetch, find six
   * meetings, disappear. Until the answer is in, the honest render is nothing.
   */
  const [loaded, setLoaded] = useState(false);
  const [dismissed, setDismissed] = useState(
    () => localStorage.getItem(DISMISSED_KEY) === "1",
  );
  const [detectionSeen, setDetectionSeen] = useState(
    () => localStorage.getItem(DETECTION_SEEN_KEY) === "1",
  );

  /**
   * Asks four independent questions, independently.
   *
   * They used to share one `try`, chained on five awaits, with the meeting
   * count last. One failure anywhere above it — an unreachable runtime, a
   * provider that will not answer — skipped the count and left it at zero,
   * and `loaded` was set in `finally` regardless. Zero meetings is exactly
   * what makes the setup card appear, so a user with months of recordings
   * got "Set up Oatmeal" because something unrelated was down.
   *
   * `allSettled`, so a failure answers only its own question.
   */
  const refresh = useCallback(async () => {
    await Promise.allSettled([
      (async () => setPermissions(await permissionsSnapshot()))(),
      (async () => {
        const [providers, current] = await Promise.all([
          providersList(),
          providerCurrent(),
        ]);
        setConfig(current);
        setHasKey(providers.find((p) => p.kind === current.kind)?.hasKey ?? false);
      })(),
      (async () => setRuntime(await runtimeState()))(),
      (async () => setMeetings((await meetingsList()).length))(),
    ]);
    setLoaded(true);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The speech model announces itself over the sidecar stream; there is no
  // command to ask "are you ready", so this listens rather than polls.
  useEffect(() => {
    const handle = onSidecarEvent((event) => {
      if (event.kind !== "event") return;
      if (event.event.ev === "model" && event.event.state === "ready") {
        setModelReady(true);
      }
      if (event.event.ev === "permissions") {
        setPermissions({
          microphone: event.event.microphone,
          screenRecording: event.event.screen_recording,
          needsRelaunch: event.event.needs_relaunch,
        });
      }
    });
    return () => {
      void handle.then((off) => off?.());
    };
  }, []);

  const progress: Progress = readProgress({
    permissions,
    modelReady,
    config,
    hasKey,
    runtime,
    detectionSeen,
  });

  if (!loaded || !shouldShow(dismissed, meetings, progress)) {
    return null;
  }

  const step = currentStep(progress);

  function dismiss() {
    localStorage.setItem(DISMISSED_KEY, "1");
    setDismissed(true);
  }

  function acknowledgeDetection() {
    localStorage.setItem(DETECTION_SEEN_KEY, "1");
    setDetectionSeen(true);
  }

  async function askForPermissions() {
    // Starting the sidecar is what makes the prompts appear; there is nothing
    // to grant until something asks.
    try {
      await sidecarStart();
    } catch {
      /* already running */
    }
    await sidecarSend({ cmd: "permissions", request: true });
  }

  return (
    <section className="card onboarding" data-testid="onboarding">
      <div className="card-head">
        <h2>Set up Oatmeal</h2>
        <span className="empty-note">
          Step {stepNumber(step)} of {STEPS.length}
        </span>
      </div>

      {step === "permissions" && (
        <div data-testid="step-permissions">
          <p className="card-note">
            Oatmeal records two things: your microphone, and whatever your Mac is
            playing. Both are needed — one without the other captures half a
            conversation.
          </p>
          <div className="row">
            <button className="primary" onClick={() => void askForPermissions()}>
              Grant access
            </button>
            <button onClick={() => void openPrivacySettings("microphone")}>
              Open System Settings
            </button>
          </div>
          {permissions?.needsRelaunch && (
            <p className="empty-note">
              Screen Recording was granted, but macOS only hands it to a freshly
              launched app. Quit and reopen Oatmeal.
            </p>
          )}
        </div>
      )}

      {step === "speech-model" && (
        <div data-testid="step-speech-model">
          <p className="card-note">
            Transcription runs on this Mac, so the speech model has to live here. It
            downloads once, the first time you record.
          </p>
          <div className="row">
            <button className="primary" onClick={() => void sidecarStart()}>
              Download the speech model
            </button>
            <button className="link-button" onClick={() => setModelReady(true)}>
              Skip for now
            </button>
          </div>
        </div>
      )}

      {step === "provider" && (
        <div data-testid="step-provider">
          <p className="card-note">
            Summaries need a language model. <strong>Fully local</strong> keeps
            everything on this Mac and needs no account — it just downloads more. A
            cloud provider is faster and needs an API key.
          </p>
          <ModelPicker />
          <div className="row">
            <button className="link-button" onClick={() => void refresh()}>
              I have set this up
            </button>
          </div>
        </div>
      )}

      {step === "detection" && (
        <div data-testid="step-detection">
          <p className="card-note">
            Oatmeal can notice when a meeting starts — from your calendar, or when an
            app like Zoom takes the microphone.{" "}
            <strong>It never starts recording on its own.</strong> You get an offer, and
            nothing happens until you accept it.
          </p>
          <p className="empty-note">
            Both triggers are off until you turn them on, and any app you say “never” to
            is never asked about again.
          </p>
          <div className="row">
            <button className="primary" onClick={acknowledgeDetection}>
              Got it
            </button>
          </div>
        </div>
      )}

      <div className="row">
        <button className="link-button" onClick={dismiss}>
          Skip setup
        </button>
      </div>
    </section>
  );
}
