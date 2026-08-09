import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import {
  LinkHighlight,
  linkHighlightPluginKey,
  sameHighlight,
} from "../lib/linkHighlightExtension";
import StarterKit from "@tiptap/starter-kit";
import { BlockId } from "../lib/blockIdExtension";
import {
  blocksById,
  hasUnsavedChanges,
  persistableBlocks,
  reconcileBlocks,
  type EditorBlock,
} from "../lib/noteBlocks";
import { notesLoad, notesSave } from "../lib/tauri";
import type { NoteBlock } from "../types";

/** How long typing has to pause before notes are written. */
const AUTOSAVE_MS = 800;

export type SaveState = "idle" | "saving" | "saved" | "error";

interface Props {
  meetingId: string | null;
  /** Milliseconds since the meeting started; drives the block anchors. */
  elapsedMs: () => number;
  onSaveStateChange?: (state: SaveState) => void;
  /** Block ids to light up, because the transcript line they came from is hovered. */
  highlightedBlocks?: Set<string>;
  /** Fires with the block under the pointer, or null on leaving the editor. */
  onHoverBlock?: (blockId: string | null) => void;
  /**
   * How much of itself to draw.
   *
   * `card` is the harness: a bordered panel with a "Notes" label, correct when
   * it sits among other diagnostic cards. `canvas` is the document, where the
   * notes *are* the page — a label and a border there would frame the writing
   * surface as one more widget on a form.
   */
  variant?: "card" | "canvas";
}

/** Reads the editor's top-level blocks in display order. */
export function readBlocks(editor: Editor): EditorBlock[] {
  const blocks: EditorBlock[] = [];
  editor.state.doc.forEach((node) => {
    const blockId = node.attrs.blockId as string | null;
    if (!blockId) return;
    blocks.push({ blockId, text: node.textContent });
  });
  return blocks;
}

/**
 * The notepad. Sparse notes typed here are the anchors the summarizer uses, so
 * every block carries the moment it was first written (SPEC section 7).
 */
export function Notepad({
  meetingId,
  elapsedMs,
  onSaveStateChange,
  highlightedBlocks,
  onHoverBlock,
  variant = "card",
}: Props) {
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const saved = useRef<NoteBlock[]>([]);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const currentMeeting = useRef<string | null>(meetingId);
  const editorRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    onSaveStateChange?.(saveState);
  }, [saveState, onSaveStateChange]);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // Notes are prose and lists; the rest of StarterKit's toolbar-oriented
        // nodes only add ways for a meeting note to go wrong.
        heading: { levels: [2, 3] },
        codeBlock: false,
        horizontalRule: false,
      }),
      BlockId,
      LinkHighlight,
    ],
    content: "",
    editorProps: {
      attributes: {
        class: "notepad-editor",
        "data-testid": "notepad",
      },
    },
  });

  const persist = useCallback(async (target: string, blocks: NoteBlock[]) => {
    setSaveState("saving");
    try {
      await notesSave(target, blocks);
      saved.current = blocks;
      setSaveState("saved");
    } catch {
      // Left as an error rather than retried silently: unsaved notes the user
      // believes are safe is the worst outcome here.
      setSaveState("error");
    }
  }, []);

  const scheduleSave = useCallback(() => {
    if (!editor || !currentMeeting.current) return;
    const target = currentMeeting.current;

    const blocks = persistableBlocks(
      reconcileBlocks(readBlocks(editor), blocksById(saved.current), elapsedMs()),
    );
    if (!hasUnsavedChanges(blocks, saved.current)) return;

    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void persist(target, blocks), AUTOSAVE_MS);
  }, [editor, elapsedMs, persist]);

  // Load whatever is already stored whenever the open meeting changes.
  useEffect(() => {
    currentMeeting.current = meetingId;
    if (!editor) return;

    if (!meetingId) {
      saved.current = [];
      editor.commands.clearContent();
      setSaveState("idle");
      return;
    }

    let cancelled = false;
    notesLoad(meetingId)
      .then((blocks) => {
        if (cancelled) return;
        saved.current = blocks;
        editor.commands.setContent(
          blocks.length === 0
            ? ""
            : {
                type: "doc",
                content: blocks.map((block) => ({
                  type: "paragraph",
                  attrs: { blockId: block.blockId },
                  content:
                    block.text.length > 0
                      ? [{ type: "text", text: block.text }]
                      : undefined,
                })),
              },
        );
        setSaveState(blocks.length > 0 ? "saved" : "idle");
      })
      .catch(() => setSaveState("error"));

    return () => {
      cancelled = true;
    };
  }, [meetingId, editor]);

  useEffect(() => {
    if (!editor) return;
    editor.on("update", scheduleSave);
    return () => {
      editor.off("update", scheduleSave);
    };
  }, [editor, scheduleSave]);

  // A pending autosave must not be lost when the view goes away.
  useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  const placeholder = useMemo(
    () =>
      meetingId
        ? "Jot down what matters. Sparse is fine — these become the anchors for the summary."
        : "Start a recording to take notes.",
    [meetingId],
  );

  // Pushed into the editor as a decoration. The transaction carries only meta,
  // so `docChanged` stays false and the autosave is never woken by a mouse move.
  useEffect(() => {
    if (!editor) {
      return;
    }
    const next = highlightedBlocks ?? new Set<string>();
    const current = linkHighlightPluginKey.getState(editor.state) ?? new Set<string>();
    if (sameHighlight(current, next)) {
      return;
    }
    editor.view.dispatch(editor.state.tr.setMeta(linkHighlightPluginKey, next));
  }, [editor, highlightedBlocks]);

  return (
    <div
      className={variant === "canvas" ? "notepad notepad--canvas" : "notepad"}
      ref={editorRef}
      onMouseLeave={() => onHoverBlock?.(null)}
      onMouseOver={(event) => {
        if (!onHoverBlock) {
          return;
        }
        const block = (event.target as HTMLElement).closest?.("[data-block-id]");
        onHoverBlock(block?.getAttribute("data-block-id") ?? null);
      }}
    >
      <div className="notepad-head">
        {variant === "card" && <span className="notepad-label">Notes</span>}
        {/* The save state stays in both. It is the only thing here that can
            tell you your typing is not reaching the disk. */}
        <span className={`notepad-save notepad-save--${saveState}`}>
          {saveState === "saving" && "saving…"}
          {saveState === "saved" && "saved"}
          {saveState === "error" && "not saved"}
        </span>
      </div>
      {editor === null ? (
        <p className="empty-note">Loading editor…</p>
      ) : (
        <EditorContent editor={editor} />
      )}
      {variant === "card" && <p className="notepad-hint">{placeholder}</p>}
    </div>
  );
}
