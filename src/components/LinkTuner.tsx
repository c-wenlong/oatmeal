import { useEffect, useState } from "react";
import { linkParamsGet, linkParamsSet, meetingIndex } from "../lib/tauri";
import { explainLink, methodBreakdown } from "../lib/links";
import type { IndexReport, LinkParams, StoredLink, Utterance } from "../types";

interface Props {
  meetingId: string | null;
  links: StoredLink[];
  utterances: Utterance[];
  /** Called after a re-link so the parent can reload what changed. */
  onRelinked: () => void;
}

/**
 * The tuning surface G18 asks for: the weights, live, and every link the
 * current weights produced with its method and score.
 *
 * Deliberately a debug panel rather than a polished setting. The right values
 * for α and β are an empirical question that only real meetings can answer, and
 * this is the instrument for answering it — the defaults shipped in
 * `LinkParams::default()` came from `link::eval`'s corpus, which is a fixture,
 * not a substitute for the user's own data.
 */
export function LinkTuner({ meetingId, links, utterances, onRelinked }: Props) {
  const [open, setOpen] = useState(false);
  const [params, setParams] = useState<LinkParams | null>(null);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<IndexReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || params) {
      return;
    }
    linkParamsGet()
      .then(setParams)
      .catch((err: unknown) => {
        setError(String(err));
      });
  }, [open, params]);

  // α and β are a split of one budget, so moving either moves the other. Two
  // free sliders would let the pair sum to something other than 1, which
  // silently rescales every score and makes two sessions incomparable.
  function setAlpha(alpha: number) {
    setParams((current) =>
      current ? { ...current, alpha, beta: Number((1 - alpha).toFixed(2)) } : current,
    );
  }

  async function apply() {
    if (!params || !meetingId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await linkParamsSet(params);
      setReport(await meetingIndex(meetingId));
      onRelinked();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  const counts = methodBreakdown(links);
  const text = new Map(utterances.map((u) => [u.id, u.text]));

  if (!open) {
    return (
      <div className="tuner tuner--closed">
        <button className="link-button" onClick={() => setOpen(true)}>
          Tune linking ({links.length} links)
        </button>
      </div>
    );
  }

  return (
    <div className="tuner" data-testid="link-tuner">
      <div className="tuner-head">
        <span className="notepad-label">Linking</span>
        <button className="link-button" onClick={() => setOpen(false)}>
          Hide
        </button>
      </div>

      <p className="card-note">
        How each note was matched to the transcript. <strong>clock</strong> is proximity
        in time, <strong>meaning</strong> is the embedding, and <strong>cited</strong>{" "}
        is the summariser naming a line itself.
      </p>

      <p className="empty-note" data-testid="link-breakdown">
        {counts.temporal} by clock · {counts.semantic} by meaning · {counts.llm} cited
        {/* `method` records which layer *decided* a link, so zero by meaning
            says the clock already agreed — not that the embedder is missing.
            Only the index report knows that, so only it may claim it. */}
        {counts.semantic === 0 && links.length > 0 && (
          <>
            {" — "}
            meaning changed no rankings here; the clock picked the same lines.
          </>
        )}
        {report?.degraded && (
          <>
            {" — "}
            <strong>no embedding model reachable</strong>, so these are timestamps alone
            and the weights below will not change anything.
          </>
        )}
      </p>

      {params && (
        <div className="tuner-controls">
          <label className="tuner-row">
            <span>
              Time vs meaning (α {params.alpha.toFixed(2)} / β {params.beta.toFixed(2)})
            </span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={params.alpha}
              aria-label="weight of time against meaning"
              onChange={(e) => setAlpha(Number(e.target.value))}
            />
          </label>

          <label className="tuner-row">
            <span>Look back {(params.lookBackMs / 1000).toFixed(0)}s</span>
            <input
              type="range"
              min={5000}
              max={120000}
              step={5000}
              value={params.lookBackMs}
              aria-label="how far back to look"
              onChange={(e) =>
                setParams((c) => (c ? { ...c, lookBackMs: Number(e.target.value) } : c))
              }
            />
          </label>

          <label className="tuner-row">
            <span>Look ahead {(params.lookAheadMs / 1000).toFixed(0)}s</span>
            <input
              type="range"
              min={0}
              max={60000}
              step={5000}
              value={params.lookAheadMs}
              aria-label="how far ahead to look"
              onChange={(e) =>
                setParams((c) =>
                  c ? { ...c, lookAheadMs: Number(e.target.value) } : c,
                )
              }
            />
          </label>

          <label className="tuner-row">
            <span>Minimum score {params.minScore.toFixed(2)}</span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={params.minScore}
              aria-label="minimum score to keep a link"
              onChange={(e) =>
                setParams((c) => (c ? { ...c, minScore: Number(e.target.value) } : c))
              }
            />
          </label>

          <label className="tuner-row">
            <span>At most {params.maxPerNote} links per note</span>
            <input
              type="range"
              min={1}
              max={10}
              step={1}
              value={params.maxPerNote}
              aria-label="maximum links per note"
              onChange={(e) =>
                setParams((c) => (c ? { ...c, maxPerNote: Number(e.target.value) } : c))
              }
            />
          </label>

          <div className="row">
            <button className="primary" onClick={apply} disabled={busy || !meetingId}>
              {busy ? "Re-linking…" : "Apply and re-link"}
            </button>
          </div>
        </div>
      )}

      {report && (
        <p className="empty-note">
          Embedded {report.embedded}, wrote {report.links} links.
          {report.degraded && ` Timestamps only — ${report.degraded}`}
        </p>
      )}
      {error && <p className="empty-note">{error}</p>}

      <ul className="tuner-links">
        {links.map((link) => (
          <li key={`${link.noteBlockId}:${link.utteranceId}:${link.method}`}>
            <span className={`log-tag log-tag--${link.method}`}>
              {explainLink(link)}
            </span>
            <span className="tuner-link-text">
              {text.get(link.utteranceId) ?? `utterance ${link.utteranceId}`}
            </span>
          </li>
        ))}
        {links.length === 0 && (
          <li className="empty-note">
            No links yet. Stop a recording, or press Apply to link this meeting now.
          </li>
        )}
      </ul>
    </div>
  );
}
