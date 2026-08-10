#!/usr/bin/env python3
"""Judge a transcript without a reference transcript.

Real meetings come with no ground truth, so quality is inferred from what is
visible in the text when ASR fails:

- **Speech rate.** Conversational English runs about 110-180 wpm. Far below
  means audio was dropped; far above means filler was invented.
- **Repetition.** Whisper's signature failure is looping a phrase. A high
  repeated-trigram share catches it, where word count alone does not.
- **Vocabulary.** A degenerate transcript reuses a small vocabulary, and the
  type-token ratio collapses well before a reader would notice.
- **Cross-model agreement.** Where two independently sized models produce the
  same words, both are probably right; where they diverge, the smaller one is
  usually the one that is wrong. This is a pointer at what to read, not a score.

None of these proves a transcript is correct. They catch the ways it goes
wrong, which is a different and more achievable claim.
"""
import json
import pathlib
import re
import sys
from collections import Counter
from difflib import SequenceMatcher

# Outside this band, something is wrong with the audio or the model — not with
# the speaker. Slow, careful dictation still clears 90.
PLAUSIBLE_WPM = (90, 200)
# Above this share of repeated trigrams, look for a loop. Genuinely repetitive
# material (a training script, a recited agenda) can reach 0.2 honestly, so
# this flags for reading rather than fails.
REPEAT_SUSPECT = 0.25


def words(text: str) -> list[str]:
    return re.findall(r"[a-z']+", text.lower())


def repeated_trigram_share(ws: list[str]) -> float:
    if len(ws) < 3:
        return 0.0
    grams = Counter(tuple(ws[i : i + 3]) for i in range(len(ws) - 2))
    return sum(c for c in grams.values() if c > 1) / max(len(ws) - 2, 1)


def analyse(path: pathlib.Path, audio_seconds: float) -> dict:
    ws = words(path.read_text())
    minutes = audio_seconds / 60 or 1
    return {
        "words": len(ws),
        "wpm": round(len(ws) / minutes, 1),
        "type_token": round(len(set(ws)) / max(len(ws), 1), 3),
        "repeat3": round(repeated_trigram_share(ws), 3),
    }


def bench_json(err: pathlib.Path) -> dict:
    match = re.search(r"BENCH (\{.*\})", err.read_text())
    return json.loads(match.group(1)) if match else {}


def main(out_dir: str) -> int:
    out = pathlib.Path(out_dir)
    rows, concerns = [], []
    for err in sorted(out.glob("*__small.en.err")):
        slug = err.name.replace("__small.en.err", "")
        bench = bench_json(err)
        if not bench:
            continue
        small = out / f"{slug}__small.en.txt"
        row = {"slug": slug, "minutes": round(bench["audioSeconds"] / 60, 1),
               "rtf": round(bench["realtimeFactor"], 1),
               **analyse(small, bench["audioSeconds"])}
        tiny = out / f"{slug}__tiny.en.txt"
        if tiny.exists():
            row["agree_tiny"] = round(
                SequenceMatcher(None, words(small.read_text()), words(tiny.read_text())).ratio(), 3
            )
        rows.append(row)

        if not PLAUSIBLE_WPM[0] <= row["wpm"] <= PLAUSIBLE_WPM[1]:
            concerns.append(f"{slug}: {row['wpm']} wpm is outside {PLAUSIBLE_WPM}")
        if row["repeat3"] > REPEAT_SUSPECT:
            concerns.append(f"{slug}: repeated trigrams {row['repeat3']} — read it for a loop")

    header = f"{'meeting':26} {'min':>5} {'wpm':>6} {'ttr':>6} {'rep3':>6} {'RTF':>6} {'agree':>6}"
    print(header)
    for r in rows:
        print(f"{r['slug'][:26]:26} {r['minutes']:5} {r['wpm']:6} {r['type_token']:6} "
              f"{r['repeat3']:6} {r['rtf']:6} {r.get('agree_tiny', '-'):>6}")

    if rows:
        print(f"\n{len(rows)} meetings, {sum(r['minutes'] for r in rows):.0f} minutes of audio")
    # Reported rather than raised: every one of these has a benign explanation
    # often enough that failing the run would train someone to ignore it.
    print("\n".join(["", "worth reading:"] + [f"  - {c}" for c in concerns]) if concerns
          else "\nnothing outside the plausible bands")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "."))
