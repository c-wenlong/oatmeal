import { useState } from "react";
import { meetingCreate } from "../lib/tauri";

/**
 * `+ New note`.
 *
 * The library had no way to begin anything. The only entry point was the record
 * control, which starts a *capture* — not the same act as opening a page to
 * type into, and impossible without a working microphone. Someone reaching for
 * a `+` wants a document.
 *
 * Recording remains a separate, later decision: press record once the note is
 * open, or never.
 */
export function NewMeetingButton({
  onCreated,
}: {
  onCreated: (meetingId: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function create() {
    setBusy(true);
    setError(null);
    try {
      onCreated(await meetingCreate());
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <button className="newnote" disabled={busy} onClick={() => void create()}>
        + New note
      </button>
      {error && <span className="empty-note">{error}</span>}
    </>
  );
}
