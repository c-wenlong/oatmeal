import type { ReactNode } from "react";

/**
 * The settings row, after Granola's Preferences screen.
 *
 * A section is one rounded group; inside it, rows share a dashed hairline that
 * starts after the icon column so the icons read as a column rather than as
 * decoration on each line. Every row is the same shape — icon, title, subtitle,
 * one control hard right — which is what makes a long settings page scannable
 * instead of merely tidy.
 *
 * This wraps the existing cards rather than rewriting them. Each card still
 * owns its own behaviour and tests; what it loses is its frame.
 */

/** Line icons, drawn rather than imported: six shapes is not a dependency. */
export const ICONS = {
  microphone:
    "M12 3a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3ZM5 11a7 7 0 0 0 14 0M12 18v3",
  eye: "M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6Z M12 9a3 3 0 1 1 0 6 3 3 0 0 1 0-6Z",
  calendar: "M4 6h16v14H4z M8 3v4 M16 3v4 M4 10h16",
  sparkle: "M12 3l2 6 6 2-6 2-2 6-2-6-6-2 6-2z",
  share: "M4 12v7h16v-7 M12 3v12 M8 7l4-4 4 4",
  lock: "M6 11h12v9H6z M9 11V8a3 3 0 0 1 6 0v3",
  info: "M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z M12 11v5 M12 8h.01",
} as const;

export type IconName = keyof typeof ICONS;

export function SettingsIcon({ name }: { name: IconName }) {
  return (
    <span className="settings-icon" aria-hidden="true">
      <svg
        viewBox="0 0 24 24"
        width="16"
        height="16"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        {ICONS[name].split(" M").map((d, i) => (
          <path key={i} d={i === 0 ? d : `M${d}`} />
        ))}
      </svg>
    </span>
  );
}

/** A group of rows under one section label. */
export function SettingsGroup({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <section className="settings-block">
      <h2 className="settings-section">{label}</h2>
      <div className="settings-group">{children}</div>
    </section>
  );
}

export function SettingsRow({
  icon,
  children,
}: {
  icon: IconName;
  children: ReactNode;
}) {
  return (
    <div className="settings-row">
      <SettingsIcon name={icon} />
      <div className="settings-row-body">{children}</div>
    </div>
  );
}
