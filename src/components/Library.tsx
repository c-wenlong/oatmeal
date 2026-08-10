import { useCallback, useEffect, useState } from "react";
import { meetingsList } from "../lib/tauri";
import type { MeetingSummary } from "../types";

/**
 * The library: every meeting, grouped by the day it happened.
 *
 * This is the screen the app has never had. Until now the only way to reach a
 * meeting was search — which requires already knowing what was said in it, and
 * so is useless for "that vendor call, some time last month".
 *
 * Deliberately not a card. Per docs/ui-teardown.md the home is a quiet list on
 * the background: date headers, a title, a time, and nothing else until you
 * hover. Every border and fill here would be one more thing between the reader
 * and the twelve words they are scanning for.
 */

/** Local calendar day, as `YYYY-MM-DD`. */
export function dayKey(ms: number): string {
  const d = new Date(ms);
  // Built from local getters rather than toISOString, which converts to UTC —
  // that would file an 8am meeting in Singapore under the previous day.
  const month = `${d.getMonth() + 1}`.padStart(2, "0");
  const day = `${d.getDate()}`.padStart(2, "0");
  return `${d.getFullYear()}-${month}-${day}`;
}

/**
 * The header for a day.
 *
 * `now` is a parameter rather than read inside, so "Today" can be tested
 * without the test passing or failing depending on when it runs.
 */
export function dayLabel(ms: number, now: number): string {
  const key = dayKey(ms);
  if (key === dayKey(now)) return "Today";
  if (key === dayKey(now - 86_400_000)) return "Yesterday";
  return new Date(ms).toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

export function timeLabel(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

/** What to call a meeting nobody named. */
export function meetingTitle(meeting: MeetingSummary): string {
  const title = meeting.title?.trim();
  if (title) return title;
  // Not "Untitled": a date is something you can actually recognise in a list.
  return `Meeting on ${new Date(meeting.startedAt).toLocaleDateString(undefined, {
    month: "long",
    day: "numeric",
  })}`;
}

export interface DayGroup {
  key: string;
  label: string;
  meetings: MeetingSummary[];
}

/** Groups meetings by local day, newest day first, newest meeting first. */
export function groupByDay(meetings: MeetingSummary[], now: number): DayGroup[] {
  const groups = new Map<string, MeetingSummary[]>();
  for (const meeting of meetings) {
    const key = dayKey(meeting.startedAt);
    const bucket = groups.get(key);
    if (bucket) bucket.push(meeting);
    else groups.set(key, [meeting]);
  }
  return [...groups.entries()]
    .sort((a, b) => b[0].localeCompare(a[0]))
    .map(([key, list]) => ({
      key,
      label: dayLabel(list[0].startedAt, now),
      meetings: [...list].sort((a, b) => b.startedAt - a.startedAt),
    }));
}

/** Meetings that are still running deserve saying so. */
export function isLive(meeting: MeetingSummary): boolean {
  return meeting.status === "recording" || meeting.status === "processing";
}

export function Library({
  onOpen,
  now = Date.now(),
}: {
  onOpen: (meetingId: string) => void;
  /**
   * Injectable so "Today" can be asserted without the test's meaning depending
   * on the day it runs. The helpers already took a clock for this reason; the
   * component reaching for `Date.now()` internally put the rot straight back.
   */
  now?: number;
}) {
  const [meetings, setMeetings] = useState<MeetingSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setMeetings(await meetingsList());
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (error) {
    return <p className="empty-note">{error}</p>;
  }
  if (!meetings) {
    return <p className="empty-note">Loading…</p>;
  }
  if (meetings.length === 0) {
    return (
      <div className="library" data-testid="library">
        <p className="library-empty">
          No meetings yet. Start recording and the first one will appear here.
        </p>
      </div>
    );
  }

  const groups = groupByDay(meetings, now);

  return (
    <div className="library" data-testid="library">
      {groups.map((group) => (
        <section key={group.key} className="library-day">
          <h2 className="library-day-label">{group.label}</h2>
          {group.meetings.map((meeting) => (
            <button
              key={meeting.id}
              className="library-row"
              onClick={() => onOpen(meeting.id)}
            >
              <span className="library-row-title">{meetingTitle(meeting)}</span>
              <span className="library-row-meta">
                {isLive(meeting) ? (
                  <span className="library-live">recording</span>
                ) : (
                  `${meeting.utteranceCount} lines`
                )}
              </span>
              <span className="library-row-time">{timeLabel(meeting.startedAt)}</span>
            </button>
          ))}
        </section>
      ))}
    </div>
  );
}
