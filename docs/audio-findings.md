# Audio findings (from the G2 spike)

Run on macOS 26.5.2, M-series, Xcode 26.2, WhisperKit `tiny.en`.

**Verdict: the architecture holds.** Dual-stream capture works, attribution works, and
WhisperKit transcribes both streams. G6/G7 can proceed on this design.

## What was proven

```
[00:36.22] [mic   ] So we'll get one for the next week.
[00:36.32] [system] So the deadline for the migration is before T.
```

The `system` line is a transcription of speech played through the speakers while
the spike ran (`say` → `afplay`). The two streams stayed independent and
correctly attributed throughout.

- **ScreenCaptureKit audio-only capture works.** No screen is recorded; video is
  configured at 2×2 @ 1fps and never read. Input arrives as
  `2 ch, 48000 Hz, Float32, deinterleaved` and resamples cleanly to 16 kHz mono.
- **`excludesCurrentProcessAudio` prevents a feedback loop.** No self-capture.
- **AVAudioEngine mic capture works** alongside it, as a genuinely separate pipeline.
- **Attribution is free.** `source` comes from which pipeline produced the samples,
  so `mic` = you and `system` = everyone else with no diarization model. This is
  the load-bearing assumption in SPEC section 4 and it is sound.
- **Screen Recording permission is required** even for audio-only, as expected.
  It's granted to the *launching* app (Terminal/Claude Code), not the binary.

## Problems found — these become G6/G7 work

### 1. Whisper hallucinates confidently on silence ⚠️

The near-silent mic channel produced a steady stream of invented content:

```
[00:30.18] [mic] (upbeat music)
[00:32.20] [mic] (audience applauding)
[00:40.21] [mic] [BLANK_AUDIO]
[00:48.19] [mic] [Music]
[00:56.19] [mic] (soft music)
```

The spike's RMS gate (`> 0.001`) is far too permissive. Left unfixed, a quiet
meeting fills the transcript with fictional stage directions — which would then be
fed to the summarizer as if real.

**G7 must:** raise the energy threshold substantially, add a proper VAD rather than
bare RMS, and filter Whisper's known silence artifacts (`[BLANK_AUDIO]`, `[Music]`,
parenthesised sound descriptions, `♪`-wrapped lines).

### 2. Speaker bleed contaminates attribution

Played through speakers rather than headphones, the mic picked up the system audio
and transcribed a garbled version of it — so the same utterance appeared on both
channels. Headphones make this a non-issue, but "laptop speakers in a room" is a
real usage mode and in-person meetings depend on the mic entirely.

**G6 should:** evaluate Apple's Voice Processing IO unit (hardware echo
cancellation) on the mic path, and consider suppressing a mic utterance that
closely matches a system utterance in the same window.

### ✅ Fixed — but not the way the note above expected

**What shipped:** `EchoSuppressor` (SidecarCore). Every settled line passes through
one gate that sees both channels in the order they arrived. A system line becomes a
candidate echo source, keyed on when it *finished* playing; a mic line is dropped
when it shares enough wording with a recent one.

Comparison is on **text, not audio** — by the time bleed has been through a speaker,
a room, a microphone and an ASR model, the waveforms have nothing left in common but
the words do. Overlap is measured as *containment* (share of the shorter line)
rather than Jaccard, because the mic copy arrives clipped and mangled and Jaccard
would punish it for the words the microphone lost.

The deliberate bias is toward keeping too much: lines under four tokens are never
suppressed. "Agreed", "the fourteenth", "sounds good" are exactly where genuine
agreement lives, and on the words alone a repeat is indistinguishable from an echo.
Deleting the user's own speech is a far worse failure than leaving a duplicate. Every
drop is logged for the same reason.

**Hardware AEC was tried and rejected — with measurements.** Enabling
`setVoiceProcessingEnabled(true)` on this machine (macOS 26.5, built-in mic) breaks
capture in three ways:

| Check | Result |
|---|---|
| Input format with AEC off | `1 ch, 48 kHz` |
| Input format with AEC on | `7 ch`–`9 ch`, **and the count varies between runs** |
| `AVAudioConverter` 7ch → mono, ch0 = 1.0, rest 0.0 | outputs **0.0** — the layout discards the mic |
| Frames captured in 3s, AEC off | **139,200** |
| Frames captured in 3s, AEC on | **0** — the tap never fires |
| Adding an output render path | `engine.start()` fails, `-10875` |

Shipping that as a default would have traded a messy transcript for a silent
microphone — losing the user's own half of every meeting. So it sits behind
`OATMEAL_MIC_AEC=1`, off by default, with the channel-0 extraction already written
for whoever verifies it on hardware where the IO unit actually initialises.

### 3. WhisperKit model downloads can land incomplete

A run ended with:

```
Transcription failed: configurationMissing("tokenizer.json")
```

The Hugging Face download had left a `.incomplete` temp file and no
`tokenizer.json`. A partially-downloaded model currently fails only at first
transcription — i.e. mid-meeting.

**G7 must:** verify model completeness at load time, not at first use, and offer a
re-download rather than failing the recording.

### 4. `tiny.en` is not good enough for real use

`"the fourteenth"` came back as `"before T"`. Expected — the spike used the
smallest model for fast iteration. Confirms SPEC's choice of `small.en` as the
shipping default.

## Notes for the real implementation

- Non-overlapping windows were fine for a spike, but boundary words were clipped.
  G7's ~5s overlap plus reconciliation is necessary, not optional.
- One shared `WhisperKit` actor served both streams without contention at 4–5s
  windows. Two model instances would double memory for no benefit.
- Diagnostics on stderr / transcript on stdout made the run trivially greppable.
  The real sidecar keeps this split.

## Status

The spike has been deleted — its capture code now lives in `sidecar/`, and each
finding above is fixed and regression-tested:

| Finding | Fix | Test |
|---|---|---|
| Whisper hallucinates on silence | `VoiceActivityDetector` gate + `TranscriptFilter` | `TranscriptQualityTests` |
| Annotations leak into speech | strip anywhere in the line, not just leading | `testStripsATrailingAnnotationFromRealSpeech` |
| Incomplete model downloads | structural check + warm-up transcription at load | `Transcriber.verify` + load path |
| Speaker bleed | **still open** — needs echo cancellation, see below |

Speaker bleed remains unaddressed: with speakers rather than headphones the mic
re-transcribes the system audio, so the same sentence appears on both channels.
Verified live at G7. The fix is Apple's Voice Processing IO on the mic path, or
suppressing a mic utterance that closely matches a system utterance in the same
window. Neither is in Phase 1's scope.
