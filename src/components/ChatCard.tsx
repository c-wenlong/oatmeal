import { useCallback, useEffect, useState } from "react";
import { chatAsk, foldersList } from "../lib/tauri";
import { timecode } from "../lib/highlight";
import type { ChatReply, Folder } from "../types";

interface Props {
  /** The meeting currently open, if any — the default scope for a question. */
  meetingId?: string | null;
  onReveal?: (meetingId: string, utteranceId: number) => void;
}

type Scope = { kind: "meeting" } | { kind: "folder"; id: string | null };

/**
 * Asking questions across meetings.
 *
 * Every claim carries its citations, and a claim that could not be traced says
 * so rather than being deleted — the same contract as the summary panels. The
 * chips resolve to a real moment in a real recording, because the citation was
 * checked against the retrieved lines before it ever reached this component.
 */
export function ChatCard({ meetingId, onReveal }: Props) {
  const [question, setQuestion] = useState("");
  const [scope, setScope] = useState<Scope>({ kind: "folder", id: null });
  const [folders, setFolders] = useState<Folder[]>([]);
  const [reply, setReply] = useState<ChatReply | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void foldersList()
      .then(setFolders)
      .catch(() => {});
  }, []);

  // Default to the open meeting when there is one: "what did we decide" almost
  // always means the conversation on screen.
  useEffect(() => {
    if (meetingId) setScope({ kind: "meeting" });
  }, [meetingId]);

  const ask = useCallback(async () => {
    if (!question.trim()) return;
    setBusy(true);
    setError(null);
    setReply(null);
    try {
      const answer = await chatAsk(
        question,
        scope.kind === "meeting" ? (meetingId ?? null) : null,
        scope.kind === "folder" ? scope.id : null,
      );
      setReply(answer);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }, [question, scope, meetingId]);

  return (
    <section className="card" data-testid="chat-card">
      <div className="card-head">
        <h2>Ask</h2>
        {busy && <span className="pill pill--pending">thinking</span>}
      </div>
      <p className="card-note">
        Answers from your transcripts only. Every claim shows the lines it came from —
        click one to hear it in context.
      </p>

      <div className="row folder-bar">
        <button
          className={scope.kind === "meeting" ? "chip chip--on" : "chip"}
          disabled={!meetingId}
          onClick={() => setScope({ kind: "meeting" })}
        >
          This meeting
        </button>
        <button
          className={
            scope.kind === "folder" && scope.id === null ? "chip chip--on" : "chip"
          }
          onClick={() => setScope({ kind: "folder", id: null })}
        >
          Everything
        </button>
        {folders.map((folder) => (
          <button
            key={folder.id}
            className={
              scope.kind === "folder" && scope.id === folder.id
                ? "chip chip--on"
                : "chip"
            }
            onClick={() => setScope({ kind: "folder", id: folder.id })}
          >
            {folder.name}
          </button>
        ))}
      </div>

      <div className="row">
        <input
          value={question}
          placeholder="what did we commit to across these calls?"
          aria-label="ask a question"
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void ask();
          }}
          style={{ flex: 1 }}
        />
        <button
          className="primary"
          disabled={busy || !question.trim()}
          onClick={() => void ask()}
        >
          Ask
        </button>
      </div>

      {reply && (
        <div className="answer" data-testid="answer">
          {reply.answer.claims.map((claim, index) => (
            <p key={index} className="claim">
              <span>{claim.text}</span>{" "}
              {claim.citations.map((citation) => (
                <button
                  key={`${citation.meetingId}:${citation.utteranceId}`}
                  className="citation"
                  title={`${citation.meetingTitle ?? "Meeting"} · ${timecode(citation.startMs)}`}
                  onClick={() => onReveal?.(citation.meetingId, citation.utteranceId)}
                >
                  {citation.meetingTitle?.trim() || "meeting"}{" "}
                  {timecode(citation.startMs)}
                </button>
              ))}
              {claim.citations.length === 0 && (
                // An uncited claim is the model's inference, not something in
                // the transcript. Saying so beats letting it look equally
                // sourced.
                <span
                  className="citation citation--none"
                  title="Not traceable to a transcript"
                >
                  uncited
                </span>
              )}
            </p>
          ))}

          {reply.answer.claims.length === 0 && (
            <p className="empty-note">The model returned nothing usable.</p>
          )}

          {reply.report.droppedCitations > 0 && (
            // Surfaced rather than swallowed: a model inventing citations is
            // worth knowing about, and hiding it would make the gate invisible.
            <p className="empty-note">
              Dropped {reply.report.droppedCitations} citation
              {reply.report.droppedCitations === 1 ? "" : "s"} that pointed at nothing.
            </p>
          )}
          <p className="empty-note">
            Answered from {reply.context.length} transcript line
            {reply.context.length === 1 ? "" : "s"}.
          </p>
        </div>
      )}

      {error && <p className="empty-note">{error}</p>}
    </section>
  );
}
