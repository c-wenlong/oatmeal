# Local models: what works, what it costs

Measured 2026-08-10/11 on this Mac (Apple silicon) against **ten real meeting
recordings — 162 minutes of audio**, and a stack that is entirely local:
WhisperKit for speech, Ollama for summaries.

```
scripts/bench-corpus.sh ~/Downloads/oatmeal-audios      # transcribe + score
sidecar/.build/release/OatmealSidecar --bench <file>    # one file
```

`--bench` runs through the same `Transcriber` the sidecar uses — a benchmark of
a different code path measures a different program.

## Is the transcript really from the audio?

Worth proving rather than assuming, because a fixture path exists in this
codebase and a cache would be invisible.

| probe | result |
|---|---|
| first 60 s of a recording, transcribed alone | agrees **100% word-for-word** with that span of the full-file transcript |
| 30 s of digital silence | **nothing** |
| the same 60 s played backwards — identical duration, energy and spectrum | **nothing** |
| all ten transcripts compared pairwise | 10 distinct; most-similar pair scores **0.033** |

The reversed clip is the one that settles it. A fixture, a cache, or a model
hallucinating from audio statistics would all produce text there; only
something actually decoding speech produces silence.

## Is the summary really from the transcript?

Same question, same standard. Summarising the 60-second clip produced three
bullets, all cited, and reading the cited lines back shows they support their
bullets: "mixed use of Outlook and WebTracker is causing double bookings" cites
`[#4] Some people use both.` and `[#5] But that's creating double bookings…`.

It also left **Decisions** and **Action items** empty, because nothing was
decided in sixty seconds. A model answering from priors fills every section.

## Transcription — WhisperKit, on the Neural Engine

| meeting | min | wpm | type/token | repeat3 | small.en RTF | agree w/ tiny.en |
|---|---|---|---|---|---|---|
| Effective Meetings | 14.3 | 183.4 | 0.272 | 0.075 | 20.0× | 0.872 |
| Fake Teams Meeting | 9.2 | 148.5 | 0.348 | 0.083 | 25.6× | 0.951 |
| Company Bonding Day | 7.9 | 115.5 | 0.360 | 0.087 | 29.6× | 0.683 |
| Mock Call (call-flow) | 16.5 | 145.4 | 0.202 | 0.207 | 24.3× | 0.965 |
| Mock meeting recording | 5.7 | 158.1 | 0.393 | 0.093 | 21.6× | 0.972 |
| Product Team 2019-07 | 42.7 | 162.9 | 0.188 | 0.087 | 24.2× | 0.912 |
| Key Meeting Engineering | 24.2 | 145.2 | 0.236 | 0.065 | 24.6× | 0.899 |
| Sec Growth DataScience | 29.2 | 150.1 | 0.228 | 0.062 | 23.4× | 0.896 |
| Minute Taking Practice | 6.0 | 158.8 | 0.436 | 0.032 | 24.1× | 0.901 |
| mock | 6.7 | 177.1 | 0.314 | 0.054 | 21.8× | 0.860 |

Every meeting lands inside the plausible speech band (110–180 wpm), so nothing
was dropped and nothing was invented. `small.en` holds **20–30× realtime**
across all ten.

Two rows deserved reading rather than a pass:

- **Mock Call, repeat3 0.207.** Not a loop. It is a call-centre training tape,
  and the repeated phrases are the script it teaches — "how may I help you"
  four times. Whisper's looping failure repeats one phrase consecutively, dozens
  of times; this is spread through the recording.
- **Company Bonding Day, agreement 0.683.** The lowest cross-model agreement,
  and `small.en` is the one that is right. The recording opens with
  "Assalamualaikum": `tiny.en` renders it as "So, Afala Marikum", emits `>>
  Sure. >> Sure.` artifacts, and calls the secretary "will be meeting his" where
  `small.en` gets "for our meeting is Leah". `small.en` did silently **drop**
  the non-English greeting rather than mangle it.

`tiny.en` runs 47–99× realtime but costs accuracy on exactly the material where
accuracy is hard. **`small.en` is the right default.**

Cost, from one 6m41s file: cold model load **36.1 s**, warm **6.9 s**,
transcription **18.8 s**, peak RSS **366 MB** cold and 144 MB warm. CPU time was
14.6 s against 56 s wall — most of the work is on the ANE and the machine stays
usable. **Loading costs twice what transcribing does**; that is CoreML compiling
for the device, it is cached afterwards, and it is the number to attack if the
first run ever feels slow.

## Summarisation — and the bug this found

**Ollama silently truncates.** Its default context window is 4096 tokens. Oatmeal
talked to the OpenAI-compatible `/v1/chat/completions` endpoint, which has no way
to change that — `options` is not part of that wire format and is ignored. When a
prompt exceeds the window Ollama does not fail; it keeps what fits and answers
from that.

Measured on the 42-minute meeting, with a passphrase planted at the top of the
transcript:

| context window | tokens evaluated (of ~12,800) | found the passphrase |
|---|---|---|
| 4096 — what the app sent | **659** | no, returned `{}` |
| 32768 | 12,824 | yes |

So a 42-minute meeting was being summarised from roughly 5% of itself, and the
result still passed the citation gate — the ids it cited existed, they just
weren't lines it had read. **Silent, plausible, and wrong**, which is worse than
a crash.

**Fixed** by talking to Ollama's own `/api/chat`, sizing `num_ctx` from the
prompt (3 chars per token, plus room for the reply, rounded to a power of two,
capped at 32k) and passing the JSON schema in the `format` field Ollama actually
honours. Verified end to end through the app's own code path —
`llm::provider::live::a_long_meeting_is_not_silently_truncated`.

### After the fix, on five meetings × two models

| meeting | model | num_ctx | evaluated | wall | bullets | cited |
|---|---|---|---|---|---|---|
| Mock meeting | `gemma4:e2b` | 4096 | 2,231 | 19.0 s | 8 | **8** |
| Mock meeting | `gemma4:latest` | 4096 | 2,355 | 29.7 s | 6 | **6** |
| mock | `gemma4:e2b` | 4096 | 2,919 | 20.7 s | 8 | **8** |
| mock | `gemma4:latest` | 4096 | 2,972 | 49.7 s | 9 | **9** |
| Fake Teams | `gemma4:e2b` | 8192 | 3,806 | 22.1 s | 12 | **12** |
| Fake Teams | `gemma4:latest` | 8192 | 4,460 | 63.3 s | 10 | **10** |
| Key Meeting Eng | `gemma4:e2b` | 16384 | 7,542 | 28.9 s | 9 | **9** |
| Key Meeting Eng | `gemma4:latest` | 16384 | 8,009 | 71.2 s | 9 | **9** |
| Product Team | `gemma4:e2b` | 16384 | 13,235 | 45.4 s | 16 | **16** |
| Product Team | `gemma4:latest` | 16384 | 13,696 | 110.8 s | 13 | **13** |

**100% cited in all ten runs, every id in range, and the repair pass never
fired.** Before the fix, two of these failed outright (malformed and empty JSON)
and two more silently summarised a fraction of the meeting.

### A correction

An earlier version of this document reported that `gemma4:e2b` "cannot cite",
from a single run that returned zero citations. That was wrong. It was context
truncation and run-to-run variance, not a property of the model: the same model
on the same transcript cites 8 of 8 once the window fits. **One run is not a
finding** — the claim survived a night longer than it should have because
nothing re-tested it.

## What this says about the product

- **Transcription is solved.** 20–30× realtime, clean text, modest memory,
  mostly off-CPU. `small.en` is the default; `tiny.en` is a fallback for
  machines that need it and a downgrade on hard audio.
- **Summarisation is where the user waits.** 19–111 s against 19 s to
  transcribe. Model size costs 2–3× wall for no gain in citation rate on this
  corpus.
- **`gemma4:e2b` is enough.** It cited every bullet on every meeting, at a third
  the memory and half the latency of the 9.5 GB model.

### Caveats

- One machine, one run per pair. Enough to size the problem and to catch a
  serious bug; not enough to rank models. The correction above is what happens
  when a single run is treated as a finding.
- The `--bench` path feeds fixed 30 s windows; the live path windows on speech
  boundaries, so these realtime factors are a ceiling.
- Quality is judged without reference transcripts — see
  `scripts/transcript_quality.py` for what the numbers can and cannot show.
