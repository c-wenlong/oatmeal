import { useEffect, useState } from "react";
import {
  detectionAnswerApp,
  detectionCandidates,
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

  return (
    <div className="popup" data-testid="detection-popup">
      <p className="popup-title">{candidateHeadline(candidate)}</p>
      <p className="popup-reason">{candidateReason(candidate)}</p>
      <div className="popup-actions">
        <button
          className="primary"
          disabled={busy}
          onClick={() => void respond(candidate.id, "start")}
        >
          Start recording
        </button>
        <button disabled={busy} onClick={() => void respond(candidate.id, "ignore")}>
          Not now
        </button>
        {candidate.bundleId && (
          <button
            className="link-button"
            disabled={busy}
            onClick={() => void respond(candidate.id, "ignore_app")}
          >
            Never for {candidate.appName ?? "this app"}
          </button>
        )}
      </div>
      {candidates.length > 1 && (
        <p className="popup-reason">{candidates.length - 1} more waiting</p>
      )}
    </div>
  );
}
