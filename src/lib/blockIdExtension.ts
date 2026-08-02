import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";

/**
 * Gives every top-level block a stable, unique `blockId` attribute.
 *
 * ProseMirror identifies nodes by position, and positions shift the moment
 * anything is inserted above them. The notepad needs identity that *doesn't*
 * shift, because each block carries a `firstTypedAtMs` the temporal linker keys
 * on — reusing a position as an identity would hand a block its neighbour's
 * anchor after every insertion.
 *
 * Ids are assigned in an `appendTransaction`, so they attach as soon as a block
 * appears and survive every later edit. A block split in two keeps its id on the
 * first half and the new half gets a fresh one, which matches how people think
 * about it: the original line is still the original line.
 */

/** Node types that carry an id. Inline nodes and marks do not need one. */
const TARGET_TYPES = ["paragraph", "heading", "listItem", "blockquote", "codeBlock"];

export interface BlockIdOptions {
  /** Injectable so tests get deterministic ids instead of random ones. */
  generateId: () => string;
}

let counter = 0;

export function defaultGenerateId(): string {
  counter += 1;
  // Random alone risks a collision across a reload; the counter makes ids
  // unique within a session and the random part unique across sessions.
  return `b${Date.now().toString(36)}-${counter}-${Math.random().toString(36).slice(2, 8)}`;
}

export const blockIdPluginKey = new PluginKey("oatmealBlockId");

export const BlockId = Extension.create<BlockIdOptions>({
  name: "oatmealBlockId",

  addOptions() {
    return { generateId: defaultGenerateId };
  },

  addGlobalAttributes() {
    return [
      {
        types: TARGET_TYPES,
        attributes: {
          blockId: {
            default: null,
            // Kept out of the rendered DOM attributes we parse back, so a copy
            // and paste of a block does not clone its identity.
            parseHTML: () => null,
            renderHTML: (attributes) =>
              attributes.blockId ? { "data-block-id": attributes.blockId } : {},
          },
        },
      },
    ];
  },

  addProseMirrorPlugins() {
    const { generateId } = this.options;

    return [
      new Plugin({
        key: blockIdPluginKey,
        appendTransaction: (_transactions, _oldState, newState) => {
          const seen = new Set<string>();
          const missing: { pos: number; attrs: Record<string, unknown> }[] = [];

          newState.doc.descendants((node, pos) => {
            if (!TARGET_TYPES.includes(node.type.name)) return;
            const id = node.attrs.blockId as string | null;
            // A duplicate means the block was split or pasted; the later copy
            // is the new one and needs its own identity.
            if (id === null || seen.has(id)) {
              missing.push({ pos, attrs: node.attrs });
            } else {
              seen.add(id);
            }
          });

          if (missing.length === 0) return null;

          const tr = newState.tr;
          for (const { pos, attrs } of missing) {
            tr.setNodeMarkup(pos, undefined, { ...attrs, blockId: generateId() });
          }
          // Not undoable on its own: pressing undo should step back through the
          // user's edits, not through bookkeeping.
          tr.setMeta("addToHistory", false);
          return tr;
        },
      }),
    ];
  },
});
