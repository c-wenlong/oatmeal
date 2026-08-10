/**
 * Keyboard chords the main window answers.
 *
 * In-app rather than global: `⌘,` means Preferences in every Mac app, and a
 * global registration would take it away from whichever app the user is
 * actually looking at. The recording hotkey is global because it is meant to
 * work while you are elsewhere; this one is not.
 */

/** The parts of a KeyboardEvent a chord is decided from. */
export interface Chord {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** How to write the chord where a person has to read it. */
export const SETTINGS_SHORTCUT = "⌘,";

/**
 * `⌘,` — Preferences, everywhere on macOS.
 *
 * The other modifiers are rejected rather than ignored. `⌥⌘,` and `⌃⌘,` are
 * chords an app can bind separately, and a predicate that waves them through
 * would open settings on a shortcut the user pressed meaning something else.
 */
export function isSettingsShortcut(event: Chord): boolean {
  return (
    event.key === "," &&
    event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey
  );
}
