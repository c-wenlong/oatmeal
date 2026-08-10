import { useEffect, useState } from "react";
import { ChatCard } from "./ChatCard";
import { RecordControl } from "./RecordControl";
import { SearchCard } from "./SearchCard";

/**
 * The bottom bar: recording on the left, asking on the right.
 *
 * Granola puts exactly these two together (docs/ui-teardown.md) — an audio
 * control and `Ask anything`, always present, never in the way. Oatmeal had
 * both, but recording was a pill of its own and asking was a card stranded on
 * the workbench behind an overflow menu, which is a worse place than where it
 * started.
 *
 * Search and Ask are the whole point of having a corpus (G24, G25). They are
 * not machinery and do not belong with the diagnostics.
 *
 * The surfaces themselves are the existing `SearchCard` and `ChatCard`, opened
 * in a sheet rather than rewritten. Their behaviour is tested and correct; what
 * was wrong was where they lived.
 */

export type AskSurface = "ask" | "search" | null;

/** What the sheet should be titled, or null when nothing is open. */
export function surfaceTitle(surface: AskSurface): string | null {
  if (surface === "ask") return "Ask";
  if (surface === "search") return "Search";
  return null;
}

export function AskBar({
  meetingId,
  onReveal,
}: {
  /** The meeting in view, so Ask is scoped to it. Null on the library, where
   *  the question is about the whole corpus. */
  meetingId: string | null;
  onReveal: (meetingId: string, utteranceId: number) => void;
}) {
  const [surface, setSurface] = useState<AskSurface>(null);

  useEffect(() => {
    if (!surface) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setSurface(null);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [surface]);

  return (
    <>
      {surface && (
        <div
          className="asksheet"
          role="dialog"
          aria-label={surfaceTitle(surface) ?? ""}
        >
          <div className="asksheet-head">
            <button
              className={
                surface === "ask" ? "asksheet-tab asksheet-tab--on" : "asksheet-tab"
              }
              onClick={() => setSurface("ask")}
            >
              Ask
            </button>
            <button
              className={
                surface === "search" ? "asksheet-tab asksheet-tab--on" : "asksheet-tab"
              }
              onClick={() => setSurface("search")}
            >
              Search
            </button>
            <button
              className="asksheet-close"
              aria-label="close"
              onClick={() => setSurface(null)}
            >
              ✕
            </button>
          </div>
          <div className="asksheet-body">
            {surface === "ask" ? (
              <ChatCard meetingId={meetingId} onReveal={onReveal} />
            ) : (
              <SearchCard onReveal={onReveal} />
            )}
          </div>
        </div>
      )}

      <div className="askbar">
        <RecordControl />
        <button className="askbar-ask" onClick={() => setSurface("ask")}>
          Ask anything
        </button>
        <button
          className="askbar-search"
          aria-label="search"
          onClick={() => setSurface("search")}
        >
          ⌕
        </button>
      </div>
    </>
  );
}
