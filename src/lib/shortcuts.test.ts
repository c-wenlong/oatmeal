import { describe, expect, it } from "vitest";
import { isSettingsShortcut, type Chord } from "./shortcuts";

const chord = (over: Partial<Chord>): Chord => ({
  key: "a",
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...over,
});

describe("isSettingsShortcut", () => {
  it("is command and comma", () => {
    expect(isSettingsShortcut(chord({ key: ",", metaKey: true }))).toBe(true);
  });

  it("is not a comma on its own", () => {
    // Otherwise typing a comma into a note would open settings.
    expect(isSettingsShortcut(chord({ key: "," }))).toBe(false);
  });

  it("is not control and comma", () => {
    // ⌃, is a different chord, and on macOS it is not Preferences.
    expect(isSettingsShortcut(chord({ key: ",", ctrlKey: true }))).toBe(false);
  });

  it("does not fire on a chord that merely contains it", () => {
    // ⌥⌘, and ⇧⌘, are separate bindings an app may own; opening settings on
    // them would answer a keystroke the user aimed somewhere else.
    expect(isSettingsShortcut(chord({ key: ",", metaKey: true, altKey: true }))).toBe(
      false,
    );
    expect(isSettingsShortcut(chord({ key: ",", metaKey: true, shiftKey: true }))).toBe(
      false,
    );
    expect(isSettingsShortcut(chord({ key: ",", metaKey: true, ctrlKey: true }))).toBe(
      false,
    );
  });

  it("ignores other keys held with command", () => {
    expect(isSettingsShortcut(chord({ key: "k", metaKey: true }))).toBe(false);
    expect(isSettingsShortcut(chord({ key: "<", metaKey: true }))).toBe(false);
  });
});
