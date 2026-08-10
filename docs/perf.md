# Local model performance

Measured 2026-08-10 on this Mac (Apple silicon), against `mock.mp3` — a real
6m41s meeting recording, 88 transcript lines, 1,204 words.

Reproduce the transcription half with:

```
sidecar/.build/release/OatmealSidecar --bench <file>
```

Numbers go to stderr as one `BENCH {...}` line; the transcript goes to stdout.
`OATMEAL_ASR_MODEL` picks the model, `OATMEAL_BENCH_WINDOW` the window size.
It runs through the same `Transcriber` the sidecar uses — a benchmark of a
different code path measures a different program.

## Transcription — WhisperKit, on the Neural Engine

| model | cold load | warm load | transcribe | vs realtime | peak RSS |
|---|---|---|---|---|---|
| `small.en` | 36.1 s | 6.9 s | 18.8–22.2 s | **18–21×** | 366 MB cold, 144 MB warm |
| `tiny.en` | 13.6 s | — | 4.7 s | **85×** | 152 MB |

**Transcription is not the problem.** `small.en` runs a six-minute meeting in
nineteen seconds and the text is clean — punctuated, correctly cased, names and
product terms intact. Live, it uses about 5% of one realtime budget, so the
headroom for a second stream, an embedder and a summariser is real rather than
hoped for.

CPU time was 14.6 s user against 56 s wall, so most of the work is on the ANE
and the machine stays usable. **Load dominates the first recording**: 36 s cold
against 19 s of transcription. That is CoreML compiling for the device, it is
cached afterwards, and it is the number to attack if the first run ever feels
slow — not the transcription.

## Summarisation — Ollama, with the panel schema the app actually sends

| model | resident | warm wall | tokens/s | bullets | **cited** |
|---|---|---|---|---|---|
| `gemma4:e2b` | 1.69 GB | 23 s | 46.6 | 12 | **0 (0%)** |
| `gemma4:latest` | 9.49 GB | 51 s | — | 7 | **7 (100%)** |

**The small model cannot cite.** Both produce well-formed JSON and readable
bullets; the schema is honoured in both cases. But `gemma4:e2b` returns an empty
`sourceUtterances` for every bullet, so every bullet arrives uncited — and
citation is the whole point of the panel. `gemma4:latest` cites all of them, and
every id it produced was inside the transcript's range.

The app degrades honestly here rather than lying: an uncited bullet keeps its
text and is marked *uncited* instead of being deleted. So `e2b` produces a
usable summary and a useless audit trail.

**Both are slower than transcription by an order of magnitude.** A six-minute
meeting transcribes in 19 s and summarises in 23–51 s. Summarisation, not ASR,
is what a user waits for.

### Caveats

- One recording, one machine, one run each. Enough to size the problem, not to
  rank models.
- Loading the summariser is a large one-off: 40 s cold against 23 s warm for
  `e2b`, 67 s against 51 s for `latest`.
- The `--bench` path feeds fixed 30 s windows. The live path windows on speech
  boundaries, so realtime factors here are a ceiling, not a promise.
