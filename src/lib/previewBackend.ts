import type {
  CalendarSource,
  DetectionSettings,
  FlowOutcome,
  GcalSettings,
  MeetingSummary,
  UpdateStatus,
} from "../types";

/**
 * A stand-in Rust core, for running the UI in a plain browser.
 *
 * Tauri has no web target: the app is a native webview around a Rust binary,
 * and in a browser `invoke` throws because `__TAURI_INTERNALS__` does not
 * exist. Every screen would sit on "Loading…" forever. This fills that gap so
 * the interface can be opened, clicked and screenshotted without building and
 * installing the desktop app for every CSS change.
 *
 * **What this can and cannot prove.** It exercises the UI and its state: what
 * each screen shows, what a switch does to the next render, which states are
 * reachable, what an error looks like. It proves nothing below the boundary —
 * not the Rust core, not EventKit, not the sidecar, not the OAuth token
 * exchange, not the Keychain. A green browser session is not evidence that
 * connecting a calendar works; it is evidence that the screen for connecting
 * one behaves.
 *
 * The state is deliberately **mutable and in-memory**, so a switch flipped here
 * stays flipped for the session. A backend that forgot every write would make
 * every state test a test of the first render only.
 */

/** Whether the UI is running in a browser rather than inside Tauri. */
export function browserPreview(): boolean {
  // Dev builds only. A production bundle that could fall back to fixtures is a
  // production bundle that can silently show invented data.
  if (!import.meta.env.DEV) return false;
  return !("__TAURI_INTERNALS__" in globalThis);
}

/**
 * What the next Connect should do, from `?connect=<reason>`.
 *
 * The failure screens are the ones worth looking at and the ones hardest to
 * reach — reproducing a real `access_denied` means clicking Deny on Google's
 * consent screen. Naming the reason in the URL makes each one a link.
 */
export function connectOutcomeFrom(search: string): FlowOutcome {
  const reason = new URLSearchParams(search).get("connect");
  if (!reason || reason === "ok") return { connected: true, reason: null };
  return { connected: false, reason };
}

/** Which scenario the fixtures start in. Set with `?preview=<name>`. */
export type Scenario = "default" | "fresh" | "google-connected" | "no-calendars";

export function scenarioFrom(search: string): Scenario {
  const asked = new URLSearchParams(search).get("preview");
  const known: Scenario[] = ["default", "fresh", "google-connected", "no-calendars"];
  return known.find((name) => name === asked) ?? "default";
}

interface Store {
  detection: DetectionSettings;
  gcal: GcalSettings;
  calendars: CalendarSource[];
  meetings: MeetingSummary[];
  /** What the next `gcal_connect` should do. Drives the failure screens. */
  connectOutcome: FlowOutcome;
}

const DAY = 86_400_000;

function baseStore(now: number): Store {
  return {
    detection: {
      leadMs: 90_000,
      micEnabled: true,
      calendarEnabled: true,
      includeSoloEvents: false,
      showUpcomingInMenuBar: false,
    },
    gcal: {
      connected: false,
      clientId: null,
      hasClientSecret: false,
      enabled: false,
    },
    calendars: [
      { id: "work", title: "Work", color: "#3b82f6", visible: true },
      { id: "personal", title: "Personal", color: "#22c55e", visible: true },
      { id: "birthdays", title: "Birthdays", color: "#ec4899", visible: false },
    ],
    meetings: [
      {
        id: "m1",
        title: "Vendor call",
        startedAt: now - 2 * 3600_000,
        endedAt: now - 3000_000,
        status: "complete",
        audioPath: null,
        utteranceCount: 214,
      },
      {
        id: "m2",
        title: "Design review",
        startedAt: now - DAY,
        endedAt: now - DAY + 1800_000,
        status: "complete",
        audioPath: null,
        utteranceCount: 88,
      },
    ],
    connectOutcome: connectOutcomeFrom(globalThis.location?.search ?? ""),
  };
}

function forScenario(scenario: Scenario, now: number): Store {
  const store = baseStore(now);
  switch (scenario) {
    case "fresh":
      // First launch: nothing granted, nothing recorded, nothing connected.
      store.detection = {
        ...store.detection,
        micEnabled: false,
        calendarEnabled: false,
      };
      store.meetings = [];
      store.calendars = [];
      return store;
    case "google-connected":
      store.gcal = {
        connected: true,
        clientId: "123-abc.apps.googleusercontent.com",
        hasClientSecret: true,
        enabled: false,
      };
      store.calendars = [
        ...store.calendars,
        {
          id: "google:primary",
          title: "Google Calendar",
          color: "#4285f4",
          visible: false,
        },
      ];
      return store;
    case "no-calendars":
      // Detection on, EventKit silent — the state that looks like a bug.
      store.calendars = [];
      return store;
    default:
      return store;
  }
}

let store: Store | null = null;

function state(): Store {
  if (!store) {
    store = forScenario(
      scenarioFrom(globalThis.location?.search ?? ""),
      Date.parse("2026-08-10T14:00:00Z"),
    );
  }
  return store;
}

/** Drops the session's state. Only for tests. */
export function resetPreview(): void {
  store = null;
}

function arg<T>(args: Record<string, unknown> | undefined, key: string): T {
  return args?.[key] as T;
}

/** The command table. Each entry is one Rust command, answered from `state()`. */
function handlers(): Record<string, (a?: Record<string, unknown>) => unknown> {
  const s = state();
  return {
    detection_settings: () => s.detection,
    detection_set_settings: (a) => {
      s.detection = arg<DetectionSettings>(a, "settings");
    },
    detection_rules_list: () => [],
    detection_pending: () => [],

    calendar_sources: () => s.calendars,
    calendar_set_visible: (a) => {
      const id = arg<string>(a, "calendarId");
      const visible = arg<boolean>(a, "visible");
      // The Google row is the gcal switch wearing a calendar's clothes, exactly
      // as it is in Rust — the preview would otherwise disagree with the app
      // about the one row most likely to be tested.
      if (id === "google:primary") s.gcal = { ...s.gcal, enabled: visible };
      s.calendars = s.calendars.map((c) => (c.id === id ? { ...c, visible } : c));
    },
    calendar_reset_visibility: () => {
      s.calendars = s.calendars.map((c) => ({ ...c, visible: true }));
      s.gcal = { ...s.gcal, enabled: true };
    },
    calendar_set_display: (a) => {
      const key = arg<string>(a, "key");
      const enabled = arg<boolean>(a, "enabled");
      if (key === "includeSoloEvents") s.detection.includeSoloEvents = enabled;
      else if (key === "showUpcomingInMenuBar")
        s.detection.showUpcomingInMenuBar = enabled;
      else throw new Error(`unknown display setting '${key}'`);
    },

    calendar_access: () => ({ authorized: true, checkedAtMs: 1_770_000_000_000 }),
    sidecar_log_tail: () => [
      "1770000000000 [supervisor] spawned pid=4242 attempt=1",
      "1770000000100 [sidecar] ready 0.1.1",
      "1770000000200 [calendar] authorized=true calendars=3 events=7",
    ],
    sidecar_log_path: () =>
      "/Users/you/Library/Application Support/com.kaichen.oatmeal/sidecar.log",

    gcal_settings: () => s.gcal,
    gcal_set_client_id: (a) => {
      s.gcal = { ...s.gcal, clientId: arg<string>(a, "clientId").trim() || null };
    },
    gcal_set_client_secret: (a) => {
      s.gcal = {
        ...s.gcal,
        hasClientSecret: arg<string>(a, "clientSecret").trim() !== "",
      };
    },
    gcal_set_enabled: (a) => {
      s.gcal = { ...s.gcal, enabled: arg<boolean>(a, "enabled") };
    },
    gcal_connect: () => {
      if (s.connectOutcome.connected) s.gcal = { ...s.gcal, connected: true };
      return s.connectOutcome;
    },
    gcal_disconnect: () => {
      s.gcal = { ...s.gcal, connected: false, enabled: false };
    },

    meetings_list: () => s.meetings,
    meeting_state: () => ({ state: "idle" }),
    meeting_active: () => null,

    permissions_snapshot: () => ({
      microphone: "granted",
      screenRecording: "granted",
      needsRelaunch: false,
    }),
    providers_list: () => [
      {
        kind: "bundled",
        label: "Bundled (local)",
        defaultBaseUrl: "http://127.0.0.1:8080",
        defaultModel: "gemma",
        requiresKey: false,
        isLocal: true,
        hasKey: false,
      },
    ],
    runtime_state: () => ({ state: "ready" }),
    runtime_model_status: () => [],
    runtime_models: () => [],
    provider_current: () => ({
      id: "bundled",
      kind: "bundled",
      baseUrl: "http://127.0.0.1:8080",
      model: "gemma",
      keychainRef: null,
    }),
    notion_settings: () => ({ hasToken: false, databaseId: null, autoExport: false }),
    privacy_snapshot: () => ({
      telemetry: false,
      retention: { days: 30 },
      generations: [],
    }),
    update_check: (): UpdateStatus => ({ decision: "up_to_date" }) as UpdateStatus,
    health_check: () => ({
      appVersion: "0.1.1-preview",
      buildProfile: "browser preview",
      arch: "browser",
      os: "browser",
    }),
  };
}

/** Sets what the next Connect should do, so the failure screens are reachable. */
export function setConnectOutcome(outcome: FlowOutcome): void {
  state().connectOutcome = outcome;
}

export async function previewInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const handler = handlers()[command];
  if (!handler) {
    // Loud rather than undefined. A preview that quietly answers "nothing" to
    // an unmodelled command produces a screen that looks broken for a reason
    // that has nothing to do with the code being tested.
    throw new Error(
      `browser preview has no fixture for '${command}' — add one in previewBackend.ts`,
    );
  }
  return handler(args) as T;
}
