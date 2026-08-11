import { describe, expect, it } from "vitest";
import {
  blocksCapture,
  canPrompt,
  capabilities,
  headline,
  rowAction,
  toggleAction,
  type PermissionsSnapshot,
} from "./permissions";

const ready: PermissionsSnapshot = {
  microphone: "granted",
  screenRecording: "granted",
  needsRelaunch: false,
};

describe("blocksCapture", () => {
  it("allows recording only when everything is granted", () => {
    expect(blocksCapture(ready)).toBe(false);
  });

  it("blocks when either capability is missing", () => {
    // Half a transcript is worse than an honest refusal.
    expect(blocksCapture({ ...ready, microphone: "denied" })).toBe(true);
    expect(blocksCapture({ ...ready, screenRecording: "denied" })).toBe(true);
    expect(blocksCapture({ ...ready, microphone: "undetermined" })).toBe(true);
  });

  it("blocks on a stale grant even though both read granted", () => {
    expect(blocksCapture({ ...ready, needsRelaunch: true })).toBe(true);
  });
});

describe("canPrompt", () => {
  it("is true only while something is undetermined", () => {
    expect(canPrompt({ ...ready, microphone: "undetermined" })).toBe(true);
    // Once macOS records a denial the prompt never reappears, so offering to
    // ask again would be a button that does nothing.
    expect(canPrompt({ ...ready, microphone: "denied" })).toBe(false);
    expect(canPrompt(ready)).toBe(false);
  });
});

describe("headline", () => {
  it("names relaunch as the fix when the grant is stale", () => {
    const text = headline({ ...ready, needsRelaunch: true });
    expect(text).toMatch(/relaunch/i);
  });

  it("prefers the relaunch message over a generic block message", () => {
    // needsRelaunch also makes blocksCapture true; the specific advice must win.
    const text = headline({
      microphone: "granted",
      screenRecording: "granted",
      needsRelaunch: true,
    });
    expect(text).not.toMatch(/blocked until/i);
  });

  it("says it can ask when something is still undetermined", () => {
    expect(headline({ ...ready, microphone: "undetermined" })).toMatch(
      /needs permission/i,
    );
  });

  it("says settings are required once denied", () => {
    expect(headline({ ...ready, microphone: "denied" })).toMatch(/blocked until/i);
  });

  it("confirms readiness when nothing is missing", () => {
    expect(headline(ready)).toMatch(/ready/i);
  });
});

describe("capabilities", () => {
  it("returns both capabilities with their settings panes", () => {
    const views = capabilities(ready);
    expect(views.map((v) => v.pane)).toEqual(["microphone", "screen_recording"]);
  });

  it("explains why screen recording is needed for audio", () => {
    // Users reasonably suspect screen capture; the copy has to defuse that.
    const screen = capabilities(ready).find((v) => v.pane === "screen_recording")!;
    expect(screen.reason).toMatch(/never records your screen/i);
  });

  it("offers a remedy for every non-granted state and none when granted", () => {
    expect(capabilities(ready).every((v) => v.remedy === null)).toBe(true);

    for (const state of ["denied", "undetermined"] as const) {
      const views = capabilities({ ...ready, microphone: state });
      const mic = views.find((v) => v.pane === "microphone")!;
      expect(mic.remedy, `no remedy for ${state}`).toBeTruthy();
    }
  });

  it("marks undetermined as promptable and denied as not", () => {
    const undet = capabilities({ ...ready, microphone: "undetermined" })[0];
    const denied = capabilities({ ...ready, microphone: "denied" })[0];
    expect(undet.promptable).toBe(true);
    expect(denied.promptable).toBe(false);
    expect(denied.tone).toBe("err");
  });
});

describe("rowAction", () => {
  it("asks macOS while a prompt can still appear", () => {
    expect(rowAction({ pane: "microphone", promptable: true }, false)).toBe("prompt");
  });

  it("sends a denied microphone to Settings", () => {
    // macOS never re-shows the prompt, so asking would do nothing visible.
    expect(rowAction({ pane: "microphone", promptable: false }, false)).toBe(
      "settings",
    );
  });

  it("tries screen recording once, since it cannot say whether it was asked", () => {
    // CoreGraphics reports only granted/denied. Refusing to prompt would send
    // a first-run user to System Settings for something one dialog would fix.
    expect(rowAction({ pane: "screen_recording", promptable: false }, false)).toBe(
      "prompt",
    );
  });

  it("stops offering once the ask changed nothing", () => {
    // The prompt did not appear, so it never will; Settings is the only route
    // left and a second Allow would be a button that does nothing.
    expect(rowAction({ pane: "screen_recording", promptable: false }, true)).toBe(
      "settings",
    );
  });
});

describe("toggleAction", () => {
  const cap = (over: Record<string, unknown>) =>
    ({ pane: "microphone", promptable: false, state: "denied", ...over }) as never;

  it("turning it on asks macOS while a prompt can appear", () => {
    expect(toggleAction(cap({ state: "undetermined", promptable: true }), false)).toBe(
      "prompt",
    );
  });

  it("turning it off can only ever be Settings", () => {
    // No API revokes an app's own grant. A switch that silently refused to
    // move would be worse than one that goes where the move is possible.
    expect(toggleAction(cap({ state: "granted", promptable: false }), false)).toBe(
      "settings",
    );
  });

  it("a denied permission goes to Settings rather than a silent no-op", () => {
    expect(toggleAction(cap({ state: "denied" }), false)).toBe("settings");
  });

  it("still tries screen recording once", () => {
    // The one case CoreGraphics cannot answer, so the app asks and learns.
    expect(
      toggleAction(cap({ pane: "screen_recording", state: "denied" }), false),
    ).toBe("prompt");
  });
});
