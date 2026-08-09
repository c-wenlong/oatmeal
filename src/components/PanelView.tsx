import { useCallback, useEffect, useMemo, useState } from "react";
import { panelDelete, panelGenerate, panelsList, templatesList } from "../lib/tauri";
import type { Panel, PanelContent, Template } from "../types";
import { TemplatePill } from "./TemplatePill";

interface Props {
  meetingId: string | null;
  /** Scrolls the transcript to a cited line. */
  onCitationClick?: (utteranceId: number) => void;
}

function parseContent(panel: Panel): PanelContent {
  try {
    return JSON.parse(panel.contentJson) as PanelContent;
  } catch {
    return { sections: [] };
  }
}

function formatWhen(epochMs: number): string {
  return new Date(epochMs).toLocaleString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * The generated summary.
 *
 * Regenerating adds a panel rather than replacing one, so an earlier version
 * the user preferred is never destroyed by a retry — and the transcript and
 * notes underneath are never touched at all.
 */
export function PanelView({ meetingId, onCitationClick }: Props) {
  const [templates, setTemplates] = useState<Template[]>([]);
  const [panels, setPanels] = useState<Panel[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [templateId, setTemplateId] = useState("default");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    templatesList()
      .then(setTemplates)
      .catch(() => setTemplates([]));
  }, []);

  const refresh = useCallback(async () => {
    if (!meetingId) {
      setPanels([]);
      setSelected(null);
      return;
    }
    try {
      const found = await panelsList(meetingId);
      setPanels(found);
      setSelected((current) =>
        current && found.some((p) => p.id === current)
          ? current
          : (found[0]?.id ?? null),
      );
    } catch (err: unknown) {
      setMessage(String(err));
    }
  }, [meetingId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const generate = async (chosen: string) => {
    if (!meetingId) return;
    setTemplateId(chosen);
    setMessage(null);
    setBusy(true);
    try {
      const panel = await panelGenerate(meetingId, chosen);
      // Show the new one, but keep the old: regenerating forks.
      setPanels((previous) => [panel, ...previous.filter((p) => p.id !== panel.id)]);
      setSelected(panel.id);
    } catch (err: unknown) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (panelId: string) => {
    setMessage(null);
    try {
      await panelDelete(panelId);
      await refresh();
    } catch (err: unknown) {
      setMessage(String(err));
    }
  };

  const open = useMemo(
    () => panels.find((p) => p.id === selected) ?? null,
    [panels, selected],
  );
  const content = open ? parseContent(open) : null;

  return (
    <section className="panel-view">
      <div className="panel-head">
        <TemplatePill
          templates={templates}
          templateId={open?.templateId ?? templateId}
          busy={busy}
          disabled={!meetingId}
          hasPanel={panels.length > 0}
          onGenerate={(chosen) => void generate(chosen)}
          footer={
            open && (
              <>
                <span className="tpill-provenance">
                  {open.provider ?? "unknown"}
                  {open.model ? ` · ${open.model}` : ""} ·{" "}
                  {formatWhen(open.generatedAt)}
                </span>
                <button className="tpill-item" onClick={() => remove(open.id)}>
                  Delete this version
                </button>
              </>
            )
          }
        />
      </div>

      {message && <p className="empty-note">{message}</p>}

      {panels.length > 1 && (
        <div className="panel-versions">
          {panels.map((panel) => (
            <button
              key={panel.id}
              className={
                panel.id === selected
                  ? "panel-version panel-version--open"
                  : "panel-version"
              }
              onClick={() => setSelected(panel.id)}
            >
              {templates.find((t) => t.id === panel.templateId)?.name ?? "Summary"}
              <span className="panel-version-when">
                {formatWhen(panel.generatedAt)}
              </span>
            </button>
          ))}
        </div>
      )}

      {!open && !busy && (
        <p className="empty-note">
          {meetingId
            ? "No summary yet. Generate one once the meeting has a transcript."
            : "Open or record a meeting to summarise it."}
        </p>
      )}

      {open && content && (
        <>
          {content.sections.map((section, sectionIndex) => (
            <div className="panel-section" key={`${section.heading}-${sectionIndex}`}>
              <h3>{section.heading}</h3>
              <ul>
                {section.bullets.map((bullet, bulletIndex) => (
                  <li
                    key={bulletIndex}
                    className={bullet.fromNote ? "bullet bullet--from-note" : "bullet"}
                  >
                    <span>{bullet.text}</span>
                    {bullet.fromNote && (
                      <span className="bullet-badge" title="Came from your notes">
                        note
                      </span>
                    )}
                    {bullet.sourceUtterances.map((id) => (
                      <button
                        key={id}
                        className="citation"
                        title="Jump to this line in the transcript"
                        onClick={() => onCitationClick?.(id)}
                      >
                        #{id}
                      </button>
                    ))}
                    {!bullet.sourceUtterances.length && (
                      // An uncited bullet is the model's paraphrase, not
                      // something traceable to the transcript. Saying so beats
                      // letting it look equally sourced.
                      <span
                        className="citation citation--none"
                        title="Not traceable to the transcript"
                      >
                        uncited
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </>
      )}
    </section>
  );
}
