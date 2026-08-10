import { SettingsIcon, type IconName } from "./SettingsRow";

/**
 * The settings sidebar, after Granola's Preferences window.
 *
 * One long scrolling page was fine at five rows. It is not fine now: finding
 * the Notion token means scrolling past permissions, detection, calendar and
 * models, and the only way to know what settings exist at all is to read the
 * whole thing. A sidebar turns that into a list you can see at a glance and a
 * pane you can land on directly.
 *
 * The panes are data rather than JSX so the sidebar and the content are
 * generated from one list. Two lists would let a nav item exist with nothing
 * behind it, which is the failure this shape is built to prevent.
 */

export type PaneId =
  "capture" | "detection" | "calendar" | "models" | "sharing" | "recordings" | "about";

export interface Pane {
  id: PaneId;
  /** Its name in the sidebar, and the heading of the pane itself. */
  label: string;
  icon: IconName;
  /** The sidebar heading it sits under; null for the first, unlabelled run. */
  group: string | null;
}

/**
 * Every pane, in sidebar order.
 *
 * No label here repeats a card's own heading — the pane title is the category
 * and the card heading is the row, so "Privacy › Privacy" would say nothing
 * twice. That is why the retention pane is called Recordings.
 */
export const PANES: Pane[] = [
  { id: "capture", label: "Capture", icon: "microphone", group: null },
  { id: "detection", label: "Detection", icon: "eye", group: null },
  { id: "calendar", label: "Calendar", icon: "calendar", group: null },
  { id: "models", label: "Models", icon: "sparkle", group: null },
  { id: "sharing", label: "Sharing", icon: "share", group: "Data" },
  { id: "recordings", label: "Recordings", icon: "lock", group: "Data" },
  { id: "about", label: "About", icon: "info", group: "App" },
];

export interface NavGroup {
  label: string | null;
  panes: Pane[];
}

/**
 * Groups panes for the sidebar.
 *
 * Consecutive runs, not a lookup by label: bucketing would let a pane added at
 * the bottom of the list jump silently into a group near the top because it
 * happened to share a name. Here the sidebar order is exactly the array order,
 * and a repeated label further down honestly draws a second heading.
 */
export function navGroups(panes: Pane[]): NavGroup[] {
  const groups: NavGroup[] = [];
  for (const pane of panes) {
    const last = groups[groups.length - 1];
    if (last && last.label === pane.group) last.panes.push(pane);
    else groups.push({ label: pane.group, panes: [pane] });
  }
  return groups;
}

export function SettingsNav({
  current,
  onSelect,
  onBack,
}: {
  current: PaneId;
  onSelect: (id: PaneId) => void;
  onBack: () => void;
}) {
  return (
    <nav className="settings-nav" aria-label="Settings sections">
      <button className="settings-nav-back" onClick={onBack}>
        ‹ Meetings
      </button>
      {navGroups(PANES).map((group) => (
        <div className="settings-nav-group" key={group.label ?? "top"}>
          {group.label && <h2 className="settings-nav-heading">{group.label}</h2>}
          {group.panes.map((pane) => (
            <button
              key={pane.id}
              className={
                pane.id === current
                  ? "settings-nav-item settings-nav-item--on"
                  : "settings-nav-item"
              }
              /* Marked, not merely tinted: which pane you are on has to survive
                 a screen reader and a colour-blind reader both. */
              aria-current={pane.id === current ? "page" : undefined}
              onClick={() => onSelect(pane.id)}
            >
              <SettingsIcon name={pane.icon} />
              <span>{pane.label}</span>
            </button>
          ))}
        </div>
      ))}
    </nav>
  );
}
