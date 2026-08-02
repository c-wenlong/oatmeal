import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

/**
 * jsdom implements no layout, so anything measuring geometry returns undefined
 * and throws. ProseMirror measures the selection on every transaction
 * (`scrollToSelection`), which makes the editor untestable without these.
 *
 * Zeroes are honest here: there genuinely is no layout, and no assertion in the
 * suite depends on real coordinates.
 */
const emptyRect = {
  x: 0,
  y: 0,
  top: 0,
  left: 0,
  right: 0,
  bottom: 0,
  width: 0,
  height: 0,
  toJSON: () => ({}),
} as DOMRect;

function installLayoutStubs() {
  const rectList = Object.assign([], { item: () => null }) as unknown as DOMRectList;

  if (typeof Range !== "undefined") {
    Range.prototype.getClientRects ??= () => rectList;
    Range.prototype.getBoundingClientRect ??= () => emptyRect;
  }
  Element.prototype.getClientRects ??= () => rectList;
  Element.prototype.scrollIntoView ??= () => {};

  // ProseMirror hit-tests the pointer against the document on every mouse
  // event. jsdom has no hit testing, so this throws rather than returning null.
  if (typeof document !== "undefined") {
    document.elementFromPoint ??= () => null;
  }
}

installLayoutStubs();

afterEach(() => {
  cleanup();
});
