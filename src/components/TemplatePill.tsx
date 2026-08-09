import { useEffect, useRef, useState } from "react";
import type { Template } from "../types";

/**
 * The template control, as one pill.
 *
 * The harness spent a labelled select and a large filled button on this. Granola
 * spends a pill and a 16px icon inside a menu row (docs/ui-teardown.md), which
 * is the right weight: choosing a template is something you do once, and
 * regenerating is something you do rarely and deliberately.
 *
 * Everything about *how* the summary was made — which provider, which model,
 * when, and how to delete it — lives in this menu too. G33's rule is that the
 * document says nothing about models; this is where that machinery went.
 */

/** What the pill itself reads. */
export function pillLabel(
  templates: Template[],
  templateId: string,
  busy: boolean,
): string {
  if (busy) return "Generating…";
  return templates.find((t) => t.id === templateId)?.name ?? "Summary";
}

/** The templates worth offering: every one that is not already applied. */
export function otherTemplates(templates: Template[], templateId: string): Template[] {
  return templates.filter((t) => t.id !== templateId);
}

export function TemplatePill({
  templates,
  templateId,
  busy,
  disabled = false,
  hasPanel,
  onGenerate,
  footer,
}: {
  templates: Template[];
  templateId: string;
  busy: boolean;
  /**
   * Closed entirely — no meeting to summarise. Distinct from `busy`, which is
   * temporary and says so on the pill. Without this the control looks available
   * and does nothing when pressed.
   */
  disabled?: boolean;
  /** Whether a summary already exists, which decides regenerate versus generate. */
  hasPanel: boolean;
  onGenerate: (templateId: string) => void;
  /** Provider, model, and deleting this version — machinery, not document. */
  footer?: React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: MouseEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  function choose(id: string) {
    setOpen(false);
    onGenerate(id);
  }

  return (
    <div className="tpill" ref={root}>
      <button
        className="tpill-button"
        aria-label="template"
        aria-expanded={open}
        disabled={busy || disabled}
        onClick={() => setOpen((was) => !was)}
      >
        <span className="tpill-spark" aria-hidden="true">
          ✦
        </span>
        {pillLabel(templates, templateId, busy)}
        <span className="tpill-caret" aria-hidden="true">
          ⌄
        </span>
      </button>

      {open && (
        <div className="tpill-menu" role="menu">
          <div className="tpill-current">
            <span className="tpill-current-name">
              {pillLabel(templates, templateId, false)}
            </span>
            <button
              className="tpill-regen"
              // The icon *is* the affordance. A button labelled "Regenerate"
              // next to a template name reads as the primary action of the
              // document, which it is not.
              aria-label={hasPanel ? "regenerate" : "generate"}
              onClick={() => choose(templateId)}
            >
              ↻
            </button>
          </div>

          {otherTemplates(templates, templateId).length > 0 && (
            <>
              <p className="tpill-heading">Templates</p>
              {otherTemplates(templates, templateId).map((template) => (
                <button
                  key={template.id}
                  className="tpill-item"
                  role="menuitem"
                  onClick={() => choose(template.id)}
                >
                  {template.name}
                </button>
              ))}
            </>
          )}

          {footer && <div className="tpill-footer">{footer}</div>}
        </div>
      )}
    </div>
  );
}
