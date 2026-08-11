import { useEffect, useState } from "react";
import {
  detectionAnswerApp,
  detectionCandidates,
  detectionJoin,
  detectionPendingQuestion,
  detectionRespond,
  onAppQuestion,
  onCandidates,
} from "../lib/tauri";
import type { AppQuestion, Candidate } from "../types";

/** What the offer calls the meeting. */
export function candidateHeadline(candidate: Candidate): string {
  if (candidate.title?.trim()) {
    return candidate.title.trim();
  }
  if (candidate.appName?.trim()) {
    return `${candidate.appName.trim()} call`;
  }
  return "Meeting starting";
}

/**
 * What the primary button offers.
 *
 * A calendar entry with a conferencing link can do both things at once, which
 * is what the user actually wants: joining the call and recording it are one
 * intention, and splitting them means doing the second one late, from another
 * window, after the call has started.
 */
export function primaryLabel(candidate: Candidate): string {
  return candidate.joinUrl ? "Join and record" : "Start recording";
}

/** The line under the title, saying why we think this is happening. */
export function candidateReason(candidate: Candidate): string {
  switch (candidate.source) {
    case "calendar":
      return candidate.appName
        ? `From your calendar · ${candidate.appName}`
        : "From your calendar";
    case "mic":
      return candidate.appName
        ? `${candidate.appName} started using your microphone`
        : "Something started using your microphone";
    case "manual":
      return "You asked to record";
  }
}

/**
 * The floating offer.
 *
 * The rule this exists to honour: **it only ever offers.** Nothing here starts
 * a recording without a click, and the window carries no close button, because
 * dismissing without an answer would leave the candidate queued and the user
 * unsure whether they are being recorded.
 */
export function DetectionPopup() {
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [question, setQuestion] = useState<AppQuestion | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void detectionCandidates()
      .then(setCandidates)
      .catch(() => {});
    // Read, not just listened for: this window is created by the same code that
    // raises the question, so the event has already been sent by the time
    // anything here could subscribe.
    void detectionPendingQuestion()
      .then((pending) => {
        if (pending) setQuestion(pending);
      })
      .catch(() => {});
    const candidateHandle = onCandidates(setCandidates);
    const questionHandle = onAppQuestion(setQuestion);
    return () => {
      void candidateHandle.then((off) => off?.());
      void questionHandle.then((off) => off?.());
    };
  }, []);

  async function respond(id: string, outcome: "start" | "ignore" | "ignore_app") {
    setBusy(true);
    try {
      await detectionRespond(id, outcome);
    } finally {
      setBusy(false);
    }
  }

  /** Opens the call and starts recording. Falls back to recording alone. */
  async function accept(target: Candidate) {
    setBusy(true);
    try {
      await detectionJoin(target.id, target.joinUrl ?? null);
    } finally {
      setBusy(false);
    }
  }

  async function answerApp(allow: boolean) {
    if (!question) return;
    setBusy(true);
    try {
      await detectionAnswerApp(question.bundleId, question.appName, allow);
      setQuestion(null);
    } finally {
      setBusy(false);
    }
  }

  // The one-time question takes precedence: it is the more specific thing to
  // ask, and answering it decides whether an offer should even exist.
  if (question) {
    const name = question.appName ?? question.bundleId;
    return (
      <div className="popup" data-testid="app-question">
        <p className="popup-title">Record when {name} uses the mic?</p>
        <p className="popup-reason">
          Asked once. Oatmeal will remember your answer and never ask again.
        </p>
        <div className="popup-actions">
          <button
            className="primary"
            disabled={busy}
            onClick={() => void answerApp(true)}
          >
            Always
          </button>
          <button disabled={busy} onClick={() => void answerApp(false)}>
            Never
          </button>
        </div>
      </div>
    );
  }

  const candidate = candidates[0];
  if (!candidate) {
    // The window is closed by Rust when the queue empties; this is only ever
    // seen for a frame in between.
    return <div className="popup popup--empty" />;
  }

  /* `data-tauri-drag-region` is what makes an undecorated window movable —
     without it there is no titlebar to grab and the offer is nailed to wherever
     macOS first put it. It goes on the surface, never on the buttons: a button
     that is also a drag handle swallows its own click. */
  return (
    <div className="popup" data-tauri-drag-region data-testid="detection-popup">
      <span className="popup-dot" aria-hidden="true" />
      <div className="popup-body" data-tauri-drag-region>
        <p className="popup-title" data-tauri-drag-region>
          {candidateHeadline(candidate)}
        </p>
        <p className="popup-reason" data-tauri-drag-region>
          {candidateReason(candidate)}
          {candidates.length > 1 && ` · ${candidates.length - 1} more waiting`}
        </p>
      </div>
      <div className="popup-actions">
        <button
          className="primary"
          disabled={busy}
          onClick={() => void accept(candidate)}
        >
          {primaryLabel(candidate)}
        </button>
        {/* Kept, compactly. Compressing this to a pill nearly dropped it, and
            "never ask about this app again" is the control that stops a
            detection feature training people to dismiss it on sight. */}
        {candidate.bundleId && (
          <button
            className="popup-never"
            disabled={busy}
            title={`Never for ${candidate.appName ?? "this app"}`}
            onClick={() => void respond(candidate.id, "ignore_app")}
          >
            Never for {candidate.appName ?? "this app"}
          </button>
        )}
        <button
          className="popup-dismiss"
          disabled={busy}
          aria-label="Not now"
          title="Not now"
          onClick={() => void respond(candidate.id, "ignore")}
        >
          ✕
        </button>
      </div>
    </div>
  );
}
