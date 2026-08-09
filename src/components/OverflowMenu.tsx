import { useEffect, useRef, useState } from "react";

/**
 * The `…` menu.
 *
 * Granola keeps almost nothing in its chrome and puts the rest behind one of
 * these (docs/ui-teardown.md). The point is not tidiness: every control visible
 * on a document is a claim that it matters as much as the writing, and most of
 * them do not.
 */

export interface MenuItem {
  label: string;
  onSelect: () => void;
}

export function OverflowMenu({
  items,
  label = "more",
}: {
  items: MenuItem[];
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;

    // A menu that survives a click elsewhere is a menu the user has to
    // dismiss deliberately, which is one interaction more than it is worth.
    const onPointer = (event: MouseEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };

    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="overflow" ref={root}>
      <button
        className="overflow-button"
        aria-label={label}
        aria-expanded={open}
        onClick={() => setOpen((was) => !was)}
      >
        …
      </button>
      {open && (
        <div className="overflow-menu" role="menu">
          {items.map((item) => (
            <button
              key={item.label}
              className="overflow-item"
              role="menuitem"
              onClick={() => {
                // Close first: an item that navigates would otherwise leave a
                // menu floating over the screen it moved to.
                setOpen(false);
                item.onSelect();
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
