#!/bin/bash
# Transcribe a folder of recordings with each ASR model and report quality.
#
# There is no reference transcript for real meetings, so "quality" here is the
# set of things that are visible in the text when ASR fails: an implausible
# speech rate, a collapsed vocabulary, a repeated phrase, or two independently
# sized models disagreeing. None of them proves a transcript is right; together
# they catch every failure mode we have actually seen.
#
#   scripts/bench-corpus.sh ~/Downloads/oatmeal-audios [outdir]
#
# Writes <outdir>/<slug>__<model>.txt (transcript) and .err (BENCH json).
set -euo pipefail

AUDIO_DIR="${1:?usage: bench-corpus.sh <audio-dir> [out-dir]}"
OUT="${2:-$(mktemp -d)/asr}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/sidecar/.build/release/OatmealSidecar"
MODELS="${OATMEAL_BENCH_MODELS:-small.en tiny.en}"

if [ ! -x "$BIN" ]; then
  echo "building the sidecar first…" >&2
  (cd "$ROOT/sidecar" && swift build -c release)
fi

mkdir -p "$OUT"
shopt -s nullglob
for f in "$AUDIO_DIR"/*.mp3 "$AUDIO_DIR"/*.m4a "$AUDIO_DIR"/*.wav; do
  slug=$(basename "${f%.*}" | tr -cd '[:alnum:]' | cut -c1-24)
  for model in $MODELS; do
    target="$OUT/${slug}__${model}"
    # Skipped rather than redone: a sweep of a two-hour corpus is long enough
    # that being able to resume it matters more than a guaranteed-fresh run.
    [ -s "$target.txt" ] && continue
    OATMEAL_ASR_MODEL=$model "$BIN" --bench "$f" > "$target.txt" 2> "$target.err"
    printf '%-26s %-9s %s\n' "$slug" "$model" \
      "$(grep -o '"realtimeFactor":[0-9.]*' "$target.err" || true)"
  done
done

echo
echo "transcripts in $OUT"
python3 "$ROOT/scripts/transcript_quality.py" "$OUT"
