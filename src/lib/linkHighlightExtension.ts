import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

export const linkHighlightPluginKey = new PluginKey<Set<string>>(
  "oatmealLinkHighlight",
);

/**
 * Highlights the note blocks linked to whatever transcript line is hovered.
 *
 * A ProseMirror decoration rather than a class set on the DOM directly:
 * ProseMirror owns the contenteditable subtree and rewrites it on its own
 * schedule, so a `classList.add` on a paragraph survives only until the next
 * reconciliation — long enough to look like it works and short enough to be a
 * confusing bug later.
 *
 * Decorations are presentation only. They never enter the document, so nothing
 * here marks the notepad dirty or triggers an autosave.
 */
export const LinkHighlight = Extension.create({
  name: "oatmealLinkHighlight",

  addProseMirrorPlugins() {
    return [
      new Plugin<Set<string>>({
        key: linkHighlightPluginKey,
        state: {
          init: () => new Set<string>(),
          apply(tr, current) {
            // Only a deliberate meta transaction changes the highlight; every
            // ordinary edit leaves it alone.
            const next = tr.getMeta(linkHighlightPluginKey) as Set<string> | undefined;
            return next ?? current;
          },
        },
        props: {
          decorations(state) {
            const highlighted = linkHighlightPluginKey.getState(state);
            if (!highlighted || highlighted.size === 0) {
              return DecorationSet.empty;
            }

            const decorations: Decoration[] = [];
            state.doc.forEach((node, offset) => {
              const blockId = node.attrs.blockId as string | null;
              if (blockId && highlighted.has(blockId)) {
                decorations.push(
                  Decoration.node(offset, offset + node.nodeSize, {
                    class: "note-block--linked",
                  }),
                );
              }
            });
            return DecorationSet.create(state.doc, decorations);
          },
        },
      }),
    ];
  },
});

/** True when two highlight sets hold the same ids. */
export function sameHighlight(a: Set<string>, b: Set<string>): boolean {
  if (a.size !== b.size) {
    return false;
  }
  for (const id of a) {
    if (!b.has(id)) {
      return false;
    }
  }
  return true;
}
