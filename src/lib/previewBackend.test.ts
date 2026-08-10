import { beforeEach, describe, expect, it } from "vitest";
import {
  connectOutcomeFrom,
  previewInvoke,
  resetPreview,
  scenarioFrom,
} from "./previewBackend";

beforeEach(() => resetPreview());

describe("scenarioFrom", () => {
  it("reads a known scenario", () => {
    expect(scenarioFrom("?preview=fresh")).toBe("fresh");
    expect(scenarioFrom("?preview=google-connected")).toBe("google-connected");
  });

  it("falls back rather than inventing one", () => {
    // A typo in the URL must not produce an empty app that looks like a bug.
    expect(scenarioFrom("?preview=nonsense")).toBe("default");
    expect(scenarioFrom("")).toBe("default");
  });
});

describe("connectOutcomeFrom", () => {
  it("makes each failure reachable by URL", () => {
    // Reproducing a real access_denied means clicking Deny on Google's consent
    // screen; this is the only way those screens get looked at.
    expect(connectOutcomeFrom("?connect=access_denied")).toEqual({
      connected: false,
      reason: "access_denied",
    });
  });

  it("succeeds by default", () => {
    expect(connectOutcomeFrom("")).toEqual({ connected: true, reason: null });
    expect(connectOutcomeFrom("?connect=ok")).toEqual({
      connected: true,
      reason: null,
    });
  });
});

describe("previewInvoke", () => {
  it("answers a command it models", async () => {
    await expect(previewInvoke("detection_settings")).resolves.toMatchObject({
      calendarEnabled: true,
    });
  });

  it("remembers a write", async () => {
    // A backend that forgot every write would make every state test a test of
    // the first render only.
    await previewInvoke("calendar_set_visible", { calendarId: "work", visible: false });
    const list =
      await previewInvoke<{ id: string; visible: boolean }[]>("calendar_sources");
    expect(list.find((c) => c.id === "work")?.visible).toBe(false);
  });

  it("routes the Google row to the gcal switch, as Rust does", async () => {
    // The preview disagreeing with the app about the one row most likely to be
    // tested is worse than having no preview.
    await previewInvoke("calendar_set_visible", {
      calendarId: "google:primary",
      visible: true,
    });
    await expect(previewInvoke("gcal_settings")).resolves.toMatchObject({
      enabled: true,
    });
  });

  it("refuses a command it does not model, loudly", async () => {
    // Quietly answering `undefined` produces a screen that looks broken for a
    // reason that has nothing to do with the code being tested.
    await expect(previewInvoke("some_unmodelled_command")).rejects.toThrow(
      /no fixture for 'some_unmodelled_command'/,
    );
  });
});
