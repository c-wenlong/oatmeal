# Oatmeal — Roadmap to v1

**Source of truth:** [SPEC.md](./SPEC.md). Where this doc and the spec disagree, the spec wins — fix this doc.

## Status

| Phase | Goals | State |
|---|---|---|
| 0 · Foundations | G1–G4 | ✅ **Complete** |
| 1 · Capture pipeline | G5–G8 | ✅ **Complete** |
| 2 · The meeting document | G9–G11 | ✅ **Complete** |
| 3 · Generation | G12–G15 | ✅ **Complete** |
| 4 · The differentiator | G16–G18 | ✅ **Complete** (G17 partly — see notes) |
| 5 · Autonomy | G19–G23 | ✅ **Complete** (see notes) |
| 6 · Corpus | G24–G25 | ✅ **Complete** |
| 7 · Ship | G26–G29 | ✅ **Complete** (G29 partly — see notes) |

**Speaker bleed is fixed** (G2 finding #2, carried since Phase 0). `EchoSuppressor`
drops mic lines that are the speakers coming back through the room. Hardware AEC was
tried and rejected on measurement — it produces zero captured frames on this machine —
so it sits behind `OATMEAL_MIC_AEC=1`. Details in `docs/audio-findings.md`.

Phase 0 notes:
- **G2 answered the risk question: the architecture holds.** Dual-stream capture and
  attribution work. It surfaced three real problems, now all fixed and
  regression-tested — see [audio-findings.md](./audio-findings.md). The spike itself
  has been deleted; its capture code lives in `sidecar/`.
- G17's linker inherits a dependency from G2: silence artifacts must be filtered
  before anything is linked, or notes will anchor to invented utterances. The
  `TranscriptFilter` gate now handles this.

Phase 3 notes:
- **The citation gate is the point of this phase.** Nothing a model returns is
  trusted: every utterance and note id is checked against the database and dropped
  if it does not resolve. Verified against a live model — gemma4:e2b invented
  citations on two separate runs and both were caught.
- A bullet that loses every citation keeps its text and is marked *uncited* rather
  than deleted. Removing it would silently drop content the user can verify in the
  transcript themselves.
- Regenerating forks rather than overwrites (the G15 gate default), so a version
  someone preferred survives a retry.
- Both decision gates took their documented defaults: templates are prompt +
  enforced JSON schema, and regenerate forks.
- **G13 is now complete.** It shipped partial in Phase 3 (management without
  downloading); the download was built afterwards and the done-when is met — see
  the G13 entry below for what the verification turned up.

Phase 2 notes:
- **G9 shipped without the Granola UI description** (the gate was never answered),
  so the meeting view follows SPEC §9 as written: notepad primary, transcript
  collapsible beside it. Worth revisiting once that description arrives.
- Note blocks needed a migration. 0001 keyed them by `seq`, which breaks the
  moment a line is inserted mid-document — every block below shifts and inherits
  its neighbour's `first_typed_at_ms`. Since the linker keys on exactly that
  timestamp, identity is now a stable editor-assigned `block_id`.
- The lifecycle is an explicit state machine. `Processing` is a real state: `stop`
  only *asks* the sidecar to finish, and the file is not complete until it answers.
- Rust owns the lifecycle and emits changes; the frontend mirrors them. A meeting
  started outside the record button — calendar detection in G22 — would otherwise
  leave the UI showing "idle" through a live recording.

Phase 1 notes:
- **Speaker bleed is still open.** Without headphones the mic re-transcribes system
  audio, so one sentence lands on both channels. Documented, not fixed — it needs
  echo cancellation and was out of scope here.
- Recording is a three-state engine — idle / armed / recording — so the ~60s
  pre-roll can fill without leaving the mic hot from launch.
- Meetings left mid-recording by a crash are recovered to `interrupted` at startup;
  without that they blocked the next recording forever.

---

29 sequential goals in 8 phases. Each goal states what it depends on, what to build, and a **Done when** that is observable — not "code written" but "this behaviour can be demonstrated." Work them in order. Don't start a goal whose dependency is unmet.

**Ordering principle:** the two things that can kill this project are dual-stream macOS audio capture (G2/G6/G7) and note-link quality (G17). Audio is proved on day one with a throwaway spike, before any investment in app structure. Link quality can't be proved early — it needs real meetings — so G17 is scheduled right after the app becomes usable enough to generate them.

---

## Verification log

Dated records of what was actually executed, so that a green suite is never mistaken
for evidence about the things a green suite cannot reach.

### 2026-08-06 — full suite, plus the live tests

`main` at `df92850`. Three suites, all passing: **506 Rust** (497 lib + 9 integration),
**269 frontend**, **98 Swift**.

The six `#[ignore]` tests were then run on their own (`cargo test -- --ignored
--test-threads=1`) and all six pass in 83s. They are ignored by default because they
need the network and a real model — not because they are flaky. What each established:

- The **llama.cpp pin `b10229`** downloads, extracts with its 18 symlinks intact, and
  reports `version: 10229 (c745be2a2)`, Darwin arm64.
- **Cold start to generation:** 491 MB fetched in 52.1s, server up, model replied.
- **Every model URL serves real GGUF bytes** — qwen2.5-3b 2.10 GB, qwen2.5-7b 4.68 GB.
- **Chat over a folder** returned 5 claims citing `[#1]`–`[#5]`, every id resolving:
  0 invented citations, 0 uncited claims.
- **An hour of transcript**: 721 texts embedded and 3 links written in 6.2s.
- **The α sweep reproduces the plateau**: 14/14 at α=0.2/0.3/0.4, falling to 10/14 at
  α=0.1 and 8/14 at α=1.0. The shipped α=0.3 sits at its centre, not on a cliff edge.

**What this does not prove.** The sweep still runs on the 14-case fixture with a
bag-of-words stand-in embedder, so it says the constant is well chosen *for that
corpus* and nothing about real meetings — G17's done-when is unchanged. Notion,
Google Calendar, EventKit, code signing and the menu bar item remain unverified for
the reasons recorded against their own goals.

### 2026-08-06 — the semantic layer, on 25 real meetings

`link::bench`, run against a real Granola corpus exported from Notion: 25 meetings,
576 note bullets, 4,596 transcript lines. Chance 4.6%.

| Embedder | top-1 | top-5 |
|---|---|---|
| bag-of-words stand-in (what the fixture uses) | 41.1% | 67.4% |
| `nomic-embed-text:v1.5` (what ships) | **90.1%** | **96.9%** |

The metric is meeting attribution — does a note's nearest transcript line come from
the meeting it was taken in — chosen because that label already exists and so nothing
has to be hand-labelled or can be tuned to fit.

**The corpus is not in this repo and must never be.** It is real meeting data; the
benchmark reads a path from `OATMEAL_BENCH_CORPUS`. See G17 for what this does and
does not settle — in short, it establishes that the semantic layer works on real
language, and settles nothing about the temporal layer, which the corpus cannot test
because it carries no timestamps.

---

## Decision gates

Open questions from SPEC §13, mapped to the goal they block. Answer each just before its goal, not now.

| Question | Blocks | Default if unanswered |
|---|---|---|
| Granola UI description | **G9** | Build from SPEC §9 as written |
| Template authoring: prompt-only or prompt + schema? | **G14** | Prompt + enforced JSON schema |
| Panel editing: does regenerate overwrite or fork? | **G15** | Fork — edits are never destroyed |
| In-person diarization: accept degradation? | **G7** | Accept for v1; FluidAudio deferred |
| Notion export shape + property mapping | **G26** | One page per meeting in a chosen database |

---

## Phase 0 — Foundations

### G1 · Repo hygiene and Tauri scaffold
**Depends on:** nothing
**Build:** Tauri v2 + React + TypeScript + Vite. `.gitignore`, `README.md`, MIT `LICENSE`, formatter/linter config for both Rust and TS. Directory layout: `src/` (React), `src-tauri/` (Rust), `sidecar/` (Swift package), `docs/`.
**Done when:** `pnpm tauri dev` opens a window rendering a React component, hot reload works, and `cargo clippy` + `tsc --noEmit` both pass clean.

### G2 · Audio + ASR spike ⚠️ highest risk
**Depends on:** G1
**Build:** A **standalone, throwaway** Swift CLI — no Tauri, no React. Captures system audio via ScreenCaptureKit (audio-only) and mic via AVAudioEngine as two independent streams, feeds both to WhisperKit, prints timestamped transcript lines tagged `mic` / `system` to stdout.
**Done when:** you run it, play a YouTube video while talking, and see two correctly-attributed interleaved transcript streams in the terminal.
**Why first:** this is where projects of this shape stall. If ScreenCaptureKit's audio-only mode, permission flow, or WhisperKit streaming has a blocking problem, we find out in a day rather than a month. Everything after this assumes it works. **Delete the spike once G6/G7 land** — it's a probe, not a foundation.

### G3 · Data layer
**Depends on:** G1
**Build:** `rusqlite` with a versioned migration runner. Full schema from SPEC §8. FTS5 virtual tables over `utterances.text`, `note_blocks.text`, and panel plaintext. `sqlite-vec` extension loaded for `embeddings`. DB lives in the app support directory.
**Done when:** migrations run from empty to current on launch, are idempotent on relaunch, and an integration test inserts a meeting with utterances and note blocks, then round-trips them through both FTS and vector queries.

### G4 · Sidecar contract and handshake
**Depends on:** G1, G3
**Build:** Swift package built as a Tauri v2 sidecar with the `aarch64-apple-darwin` suffix. Newline-delimited JSON over stdio per SPEC §3. At this stage the sidecar **emits scripted fake events** — no real audio. Rust side: spawn, supervise, parse, restart on crash, surface events to the frontend over Tauri's event channel.
**Done when:** the sidecar is spawned by the packaged app, fake `partial`/`final`/`level` events arrive in the React devtools console in order, and killing the sidecar process externally causes a clean restart with a logged error.
**Why fake events first:** it decouples the IPC/supervision problem from the audio problem, so G6's debugging is purely about audio.

---

## Phase 1 — Capture pipeline

### G5 · Permissions manager
**Depends on:** G4
**Build:** Detect and request Screen Recording and Microphone permission. Report status to the UI. Handle the macOS quirk that Screen Recording grants often need an app relaunch. A blocking pre-flight screen when either is missing, with a deep link to the right System Settings pane.
**Done when:** on a machine with permissions revoked, the app explains exactly what's missing, opens the correct settings pane, and recovers without a manual quit.

### G6 · Dual-stream capture, ring buffer, audio persistence
**Depends on:** G2, G5
**Build:** Port the spike's capture half into the real sidecar. Two streams stay separate. A continuous ~60s ring buffer so "record now" retroactively captures what just happened. Encode to a single two-channel AAC/Opus file; return the path and duration on stop.
**Done when:** a 10-minute recording produces a ~5MB two-channel file where channel 1 is you and channel 2 is system audio, and starting mid-conversation includes the preceding ~60s.

### G7 · WhisperKit streaming
**Depends on:** G6
**Build:** Port the spike's ASR half. Sliding ~30s windows with ~5s overlap, plus overlap reconciliation so words aren't duplicated at boundaries. Emit `partial` for the live UI and `final` once a window settles. Model download on first run with progress; `small.en` default, `tiny.en` and multilingual `small` selectable.
**Done when:** live partials appear within ~2s of speech, finals are stable and free of boundary duplication, and switching models in settings takes effect on the next recording.

### G8 · First vertical slice
**Depends on:** G3, G7
**Build:** Wire it end to end. Rust persists `final` events as `utterances`. React renders a live transcript with `You` / `Them` attribution and timestamps. Start/stop from a temporary debug button.
**Done when:** you press record, talk over a call, press stop, and the full transcript is in SQLite and re-renders correctly after an app restart. **This is the first genuinely useful build.**

---

## Phase 2 — The meeting document

### G9 · Notepad with block timing
**Depends on:** G8 · *gated on the Granola UI description*
**Build:** Block-structured editor (TipTap/ProseMirror — matches Granola's own model and gives us the panel structure for free). Every block records `first_typed_at_ms` and `last_edited_at_ms` relative to meeting start. Autosave. Notepad is the primary column; transcript is a collapsible secondary panel.
**Done when:** notes typed during a recording persist with accurate per-block timings, verifiable in the DB, and survive an app crash mid-meeting.

### G10 · Meeting lifecycle
**Depends on:** G9
**Build:** Explicit state machine — `idle → armed → recording → processing → complete` — owning the sidecar, the audio file, and DB writes. Crash recovery: a meeting left in `recording` on launch is recovered from whatever was persisted rather than lost.
**Done when:** every transition is driven through the state machine, and force-quitting mid-recording leaves a recoverable meeting with its transcript up to the crash point.

### G11 · Library
**Depends on:** G10
**Build:** Meeting list with date, title, duration. Open a past meeting into the same view as a live one, minus the recording controls. Rename, delete.
**Done when:** you can navigate a week of meetings and reopen any of them with notes and transcript intact.

---

## Phase 3 — Generation

### G12 · LLM provider layer
**Depends on:** G3
**Build:** One internal interface, OpenAI chat-completions shaped. Presets per SPEC §10 (Anthropic via thin adapter, OpenAI, OpenRouter, Ollama, LM Studio). Keys in the **macOS Keychain**, never SQLite. Settings UI: pick provider, pick model, test-connection button. Streaming responses.
**Done when:** each configured provider round-trips a test prompt, keys survive relaunch, and no key is ever written to the DB or logs.

### G13 · Bundled local inference
**Depends on:** G12
**Build:** JIT-download `llama-server` from llama.cpp GitHub releases on first use (not static-bundled). Verify the download, manage the process lifecycle, expose it as the `localhost:8080/v1` preset. Model picker with download progress.
**Done when:** a user with no API key and no Ollama install can summarize a meeting entirely offline after one guided download.
**Status:** ✅ Done, and **verified end to end against the real network** rather
than assumed. `live_a_bare_machine_can_generate_after_downloading` (ignored by
default) starts from an empty directory, downloads the server, downloads a model,
launches `llama-server`, and gets a completion back. Last run: server 1.3s, model
491 MB in 32s, model replied `"oatmeal"`.

Downloads stream to a `.part` file and resume with a range request — a 4.7 GB model
on a domestic line cannot be asked to restart. A `200` answer to a range request
discards the partial rather than appending, because splicing the head of a file onto
its middle produces corruption that survives every size check. Cancelling keeps the
bytes.

**Three things were broken and only measurement found them:**

| What | Was | Actually |
|---|---|---|
| llama.cpp asset | `…-macos-arm64.zip` | **404** — upstream ships `.tar.gz` now |
| Qwen 7B model URL | single `q4_k_m.gguf` | **404** — official repo splits it into two shards; switched to a single-file build |
| Archive extraction | files only | the release has **18 symlinks**, and `libllama-common.0.dylib` — the name the binary asks dyld for — is one of them |

The symlink one is the sharpest lesson: extracting only regular files produced a
complete-looking install whose binary died with `Library not loaded`, and the first
version of the live test *passed anyway* because its assertion was loose enough to
match a dyld error. Both were fixed; removing the symlink handling now fails the test.

Extraction is flattened deliberately (`LC_RPATH` is `@loader_path`, so the dylibs
must sit beside the binary) and refuses `..` paths and symlinks pointing outside the
runtime directory — this archive arrives over the network.

The release tag is **pinned** (`SERVER_RELEASE`) rather than resolved from "latest":
a verified build beats whatever landed upstream this morning, and the asset naming
has already changed once.

### G14 · Templates and panel generation
**Depends on:** G11, G12 · *gated on the template-authoring decision*
**Build:** Built-in templates (default summary, 1:1, standup, sales call, interview) plus user-defined ones. The summarizer receives the transcript with stable utterance IDs **and** the notes with block IDs, and returns structured output where each bullet carries `source_utterances` and optional `from_note`. **Validate every returned ID against the DB and silently drop invalid ones** — this is the anti-hallucination gate. Repair-retry path for local models that ignore the schema.
**Done when:** ending a meeting produces a panel whose every citation resolves to a real utterance, and a deliberately hallucinated ID injected in a test is dropped rather than rendered.

### G15 · Panel UI
**Depends on:** G14 · *gated on the edit/regenerate decision*
**Build:** Generated panel above the notes. Template switcher, regenerate. Citation chips that scroll the transcript to the cited utterance. Multiple panels per meeting, since panels are regenerable and the transcript is not.
**Done when:** switching templates generates a second panel without touching the transcript, the notes, or the first panel, and every chip navigates correctly.

---

## Phase 4 — The differentiator

### G16 · Local embeddings
**Depends on:** G3
**Build:** Local embedding model (bge-small or EmbeddingGemma via CoreML in the sidecar, or `fastembed-rs` in the core — benchmark both, pick on latency). Embed utterances and note blocks on meeting completion, in the background. Store via `sqlite-vec`. Backfill job for pre-existing meetings.
**Done when:** a one-hour meeting embeds in under ~30s in the background without blocking the UI, and nearest-neighbour lookup returns semantically sensible utterances.
**Status:** ✅ Done. Built as `HttpEmbedder` against any OpenAI-shaped `/embeddings`
endpoint rather than a bundled model — `nomic-embed-text:v1.5` through the local
runtime G13 already manages. Measured against the real model: **721 texts (an hour
of transcript at one line per five seconds) in 7.7s**, well inside the 30s budget
(`an_hour_of_transcript_embeds_within_the_budget`, ignored by default because CI
has no model). Indexing runs on a blocking thread after the meeting ends and
re-uses vectors it already has, so re-opening a meeting costs nothing.
Migration 0004 widened the vector index 384 → 768 to match the model.

### G17 · Layered linker ⚠️ highest-risk feature
**Depends on:** G14, G16
**Build:** All three layers per SPEC §7. Temporal candidates in `[T-45s, T+10s]`, asymmetric because you type *after* hearing. Semantic rerank, combined `α·temporal + β·semantic`. Global semantic pass to catch late notes, added as a second link when it beats the windowed best by a margin. LLM citations from G14 merged in. Every link stored with its `method` and `score`.
**Done when:** across 10 real meetings, hand-review says the top link for each note block is correct materially more often than the timestamp-only baseline. **Build the baseline first and measure against it** — otherwise there's no way to know if the semantic layer is helping or hurting.
**Status:** ⚠️ Built and measured, but **the done-when as written is not met and cannot
be met by the agent** — it requires ten of *your* real meetings and *your* judgement of
what each note refers to. What exists instead:

- The baseline was built first (`link_baseline`), and the measurement harness
  (`link::eval`) scores any labelled corpus through either linker.
- On a 14-case fixture corpus: **baseline 9/14 (64%), layered 14/14 (100%)**.
- α/β are **measured, not guessed**. The first sweep recommended α=0.0 — throw the
  clock away entirely — which turned out to be an artifact of a biased corpus: every
  case made the correct answer the topically-matching line, which bag-of-words
  matching nails. Four cases were added where the note is shorthand sharing no
  vocabulary with the transcript ("!!", "ask J re: timeline"), which is how people
  actually write. The optimum then moved to a **plateau at α=0.2–0.4**, and the
  shipped default is α=0.3 — the centre of the plateau, not the edge of a spike.
  `weighting_curve` (ignored) prints the whole curve.
- **The fixture is a harness, not evidence.** It uses a bag-of-words stand-in
  embedder and cases the author wrote.
- **The semantic layer is now measured on real meetings** (`link::bench`, added
  2026-08-06). 25 real meetings — 576 note bullets against 4,596 transcript lines —
  scored by whether a note's nearest line comes from the meeting it was taken in.
  That label is free, so nothing is hand-labelled and nothing can be tuned to fit.
  Chance is 4.6%. **The real embedder scores 90.1% top-1 / 96.9% top-5; the
  bag-of-words stand-in scores 41.1% / 67.4%.** Two conclusions: the semantic layer
  works on real meeting language, and the fixture *understates* production by better
  than a factor of two rather than flattering it.
- **What the benchmark still does not settle.** Attribution is a proxy — picking the
  right *line within* a meeting is the actual job, and a note can find the right
  meeting and the wrong line in it. The corpus is Granola's AI-written summary
  bullets: clean full sentences, the easy case, not the shorthand people type. And it
  carries **no timestamps on either side**, so the temporal layer is untouched and the
  timestamp-only baseline cannot even be computed against it. Ten meetings recorded
  *in Oatmeal* remain the only way to close the done-when, and that is yours to do.

### G18 · Linking UI and tuning
**Depends on:** G17
**Build:** Hovering a note block highlights its linked transcript spans; hovering a transcript span highlights the notes drawn from it. Note-derived summary bullets visually distinguished from transcript-only ones. A debug panel exposing α/β and showing per-link method and score.
**Done when:** the bidirectional highlight is correct and legible on a one-hour meeting, and α/β can be tuned live against a real meeting without a rebuild.
**Status:** ✅ Done, with one piece deferred. The bidirectional highlight and the
tuning panel (α/β, windows, min score, max links, plus per-link method and score)
both work; Apply re-links the open meeting live, no rebuild.

Two things worth recording:
- The note-side highlight is a **ProseMirror decoration**, not a DOM class. The first
  attempt set `classList` directly and passed its test — then ProseMirror's next
  reconciliation wiped it. Decorations are presentation-only and never mark the
  notepad dirty, so hovering cannot trip the autosave.
- Writing the hover test surfaced a **pre-existing bug from G15**: the mount path
  that restores the last meeting built its transcript lines without `utteranceId`,
  so on a cold start citation chips had nothing to scroll to. Fixed.

All three parts of the goal are built, including "note-derived summary bullets
visually distinguished from transcript-only ones": `Bullet.from_note` carries the
provenance, the prompt asks for it, `validate` drops it when the id is not a real
note block, and `PanelView` renders it as a `note` badge with an accent rule down
the side (`.bullet--from-note`). Covered by tests in `panel/content.rs`.

**Not verifiable by the agent:** the hover itself. macOS withholds Accessibility
permission here, so synthetic pointer events are dropped and the highlight cannot be
driven in the running app. It is covered by DOM tests instead, and the panel was
photographed live against a seeded meeting (12 links from the real embedder,
"9 by clock · 3 by meaning" — matching the database exactly).

**Two bugs fixed in passing:** deleting a meeting left its vectors behind
(`embeddings` is a `vec0` virtual table, so nothing cascades into it), and the
in-window links were all labelled `temporal` even when meaning decided them, which
made the stored `method` useless for judging whether the semantic layer helps.
Links are now labelled `semantic` when meaning *promoted* them past what the clock
would have picked.

---

## Phase 5 — Autonomy

### G19 · Menu bar widget and manual capture
**Depends on:** G10
**Build:** Menu bar item: recording status with elapsed time, "Record now", recent meetings list, open main window, quit. Global hotkey to start/stop. This is the manual path — always available regardless of detection.
**Done when:** a meeting can be recorded start to finish without ever opening the main window, and the hotkey works while another app is focused.

### G20 · Calendar sync
**Depends on:** G3
**Status:** ✅ Done, but **built on EventKit rather than Google/Microsoft OAuth**,
which is a deliberate departure worth flagging. macOS already syncs the user's
calendars — Google, Exchange, iCloud — and EventKit reads all of them locally. The
OAuth route would mean registering clients, shipping a client secret inside an app
whose whole premise is "nothing leaves this Mac", and storing refresh tokens. Reading
what the OS already has is less code and a better promise. The trade: someone whose
calendar is not in macOS Calendar sees nothing, which is recoverable later by adding
a provider path — shipping a secret is not.

The meeting-shaped heuristic lives in Rust (`detect::calendar`), pure and tested, so
the sidecar stays a dumb reader. All-day entries are dropped outright.

**Not verified end to end:** EventKit needs the user to grant calendar access, and
the prompt only appears for a bundled app. The fetch and heuristic are tested; the
grant is yours to give.

### G20b · Google Calendar over OAuth (added after Phase 7)

Built on request, as an **addition** to EventKit rather than a replacement: a
calendar already in Calendar.app needs no account and no token, and that path stays
the default. This covers the case EventKit cannot reach.

**PKCE with a loopback redirect, and no client secret.** Google documents the secret
as inapplicable to installed apps, and PKCE replaces what it was doing — the app
proves it started the flow by producing the pre-image of a hash it sent earlier, so
nothing confidential ships in the binary. What ships is the user's own client *id*.

Three security properties, each with a test that fails when it is removed:

- **The `state` check.** Without it, anything able to reach the loopback port could
  hand back an authorization code from a different account and it would be redeemed.
  A test drives exactly that attack and asserts the exchange never runs.
- **An empty state never matches**, so a missing value cannot pass the check.
- **The code never reaches the browser page.** Putting it in the HTML would leave it
  in the page source and in browser history.

Also: the listener binds `127.0.0.1` only (never `0.0.0.0`), the scope is
`calendar.events.readonly` rather than the broader `calendar.readonly`, and the
refresh token goes to the Keychain while the access token stays in memory.

**A real bug found by a slow test.** `accept()` blocks until something connects, so
the timeout was only consulted *between* connections — a user who closed the browser
mid-flow would have left the app waiting forever on a bound port. The test that
caught it now asserts the timeout is honoured promptly.

**Notion OAuth was investigated and rejected.** Their token exchange requires a
`client_secret` over HTTP Basic and PKCE is not supported, so a desktop app would
have to either ship an extractable secret or route every export through a proxy —
the second contradicts the promise the privacy panel makes. The integration token
stays until Notion supports PKCE.

**Shipping constraint you should know about:** `calendar.events.readonly` is a
sensitive scope. Publishing needs Google verification (brand review plus data-access
review, a justification video, a privacy policy and a verified domain). In Testing
mode it works but caps users, shows an unverified-app warning, and limits refresh
token lifetime.
**Build:** Google Calendar + Microsoft Graph OAuth, **read-only**, tokens in Keychain. Poll every ~5 min into `calendar_events`. Meeting-shaped heuristic: has a conferencing URL, or ≥2 attendees, or an explicit location. Skippable during onboarding — the app must work fully without it.
**Done when:** today's events appear in-app within 5 minutes of being created in Google Calendar, and revoking access degrades gracefully to manual + mic detection.

### G21 · Mic-activation watcher
**Depends on:** G19
**Status:** ✅ Done and **verified against a real app taking the microphone**. Uses
the audio process-object API (macOS 14.4+) — the only supported way to attribute
input to a process without private API. Both `started` and `stopped` were observed
end to end through the real sidecar with the correct bundle id.

Two things worth recording:
- `NSRunningApplication` returns nil for a process LaunchServices did not register —
  and **browsers run audio in a helper process**, so "Meet in Chrome" would have been
  undetectable. There is now a fallback that resolves the pid's executable path to
  the *outermost* enclosing `.app`: the innermost is "Google Chrome Helper", which is
  neither recognisable to a user nor stable across updates.
- Processes with no bundle id are dropped at both ends. A rule has to outlive the
  process, and a pid does not.
**Build:** Poll which processes hold the audio input device. **Nothing fires without an explicit per-app rule.** Ship the built-in allowlist (Zoom, Meet in Chrome/Safari/Arc, Teams, Slack, Discord, FaceTime, Webex). An unknown app triggers a one-time "Should Oatmeal offer to record when *X* uses the mic?" with Always / Never; Never is permanent.
**Done when:** Zoom triggers a candidate and a dictation tool like Whisperflow does not — and after choosing Never once, it never asks again.

### G22 · Detection orchestrator and popup
**Depends on:** G20, G21
**Status:** ✅ Done and **photographed working**: an unknown app took the microphone
and the floating window appeared asking "Record when MicHolder uses the mic?" with
Always / Never.

Dedup is the headline requirement and is mutation-tested — never-merge, always-merge
and source-downgrade all fail the suite. A calendar event and a mic activation within
five minutes collapse into one offer that keeps the better-informed title; two
different calendar events never merge however adjacent they are.

**Nothing here records.** Every path produces an offer that needs a click.
**Build:** One candidate queue fed by calendar, mic, and manual. Deduplicate — a calendar event and a mic activation for the same call must produce one popup, not two. Floating always-on-top window: title (from calendar when known), Start / Ignore / Ignore-this-app. Auto-dismiss after ~60s as Ignore. **Never auto-records without consent.**
**Done when:** a calendar meeting pops up at `start - lead`, joining it does not produce a second popup, Start begins recording with the calendar title and attendees pre-filled, and nothing is ever recorded unprompted.

### G23 · Detection settings
**Depends on:** G22
**Status:** ✅ Done. Both triggers ship **off**; detection watches other apps and
reads the calendar, and neither should begin because the app was launched. The
allowed/ignored columns are editable, shipped defaults are labelled as such, and a
user rule replaces the default it overrides rather than appearing twice.
**Build:** Configurable calendar lead time (default 90s). The two-column app list — allowed vs ignored — fully editable, with a way to add an app not yet seen. Master toggles per trigger source.
**Done when:** every detection behaviour from G20–G22 is reachable and reversible from the UI, with no hardcoded values left.

---

## Phase 6 — Corpus

### G24 · Folders and search
**Depends on:** G16, G11
**Build:** Folder CRUD, assign meetings to folders. Search combining FTS5 keyword and vector similarity, results grouped by meeting with matched-span previews.
**Done when:** searching a phrase you remember imperfectly from three weeks ago finds the right meeting and jumps to the right moment in the transcript.
**Status:** ✅ Done, and **verified against real transcripts with the real embedder**
(`cargo run --example trysearch`), not only against the bag-of-words stand-in the
unit tests use. Three queries over two seeded meetings:

| Query | Found | Moment |
|---|---|---|
| `two year commitment` | Vendor negotiation | 12.0s — exact line, all three words marked |
| `supplier lock-in` | Vendor negotiation | 20.0s — **no word in common**, pure semantics |
| `shrink the scope` | Platform planning | 15.0s — "cut the release scope in half" |

The two indexes are fused by **rank position, not score**: FTS5's `rank` is a
negative BM25 and cosine distance is a small positive, neither with a stable range,
so normalising and blending them would be arithmetic on incomparable units.
Reciprocal rank fusion means agreement between the indexes beats confidence from
either one.

**Two bugs the real data found that the unit tests could not.** The third query
above originally returned the *wrong meeting*:

- FTS5 ANDs terms by default, so one wrong word ("shrink") made the keyword half
  match nothing at all. A feature for half-remembered phrases cannot require every
  word to be right — terms are now ORed.
- That exposed the second: under `OR`, "the" matches every line and **earns a
  rank**, and because fusion is by rank position, BM25's sensible decision to
  weight it near zero never gets a say. Common words are now dropped from the
  query, not merely from the highlight.

Snippets return character offsets rather than markup, so a transcript cannot smuggle
HTML into the UI, and the frontend counts characters (`Array.from`) rather than
UTF-16 units — `slice` drifts by one per emoji and highlights the wrong words after
it. Deleting a folder keeps its meetings, which the confirmation says out loud.

### G25 · Chat over meetings
**Depends on:** G24
**Build:** Chat scoped to one meeting or a whole folder. Retrieval over `utterances` + `panels`, answers citing meeting and timestamp. Uses the G12 provider layer, so it works locally.
**Done when:** "what did we commit to across these calls?" over a folder of five meetings returns an answer whose every claim carries a citation that resolves.
**Status:** ✅ Done, and **verified with that exact question against a live model**
(`live_a_folder_question_is_answered_with_resolving_citations`, ignored by default).
Over five meetings with one commitment each, gemma4:e2b returned five claims —
**every citation resolved, 0 dropped, 0 uncited.**

The citation gate is the panel gate again, for the same reason: a model asked to
cite will invent plausible ids, and a chip that jumps nowhere is worse than no
answer because the user has no reason to doubt it until they click. Three mutations
all fail the suite — accepting every citation, deleting uncited claims instead of
marking them, and trusting the model's own idea of which meeting a line came from.

A claim that loses every citation is **kept and marked uncited**, never deleted:
silently removing what the model produced hides what it did, and the user can still
judge a sentence.

Retrieval reuses the G24 search rather than adding a second path — two ways to find
a line would eventually disagree, and a wrong answer stays debuggable when the
evidence is the same evidence the user could have found by searching. A single
meeting is handed over whole; retrieval over one conversation risks dropping the
line that answers the question because the question happened not to share its
words.

---

## Phase 7 — Ship

### G26 · Notion export
**Depends on:** G15 · *gated on the export-shape decision*
**Build:** Notion integration token, database picker, property mapping (title, date, duration, attendees, folder). Export the panel plus optionally the transcript. Store the Notion page ID so re-export updates rather than duplicates. Optional auto-export on meeting completion.
**Done when:** completing a meeting creates a correctly-propertied Notion page, and regenerating a panel then re-exporting updates that same page instead of creating a second one.
**Status:** ✅ Built, and the create-then-update round trip is **proven against a
real HTTP server** standing in for Notion: a second export issues no `POST /v1/pages`,
patches the same page, and clears the old body *before* writing the new one —
appending would leave two summaries stacked on one page, which reads worse than a
duplicate.

Only properties the target database actually has are sent; Notion rejects the whole
request for one unknown name, so a database without "Duration" would otherwise fail
outright instead of exporting what it can. The title column is read from the database
rather than assumed to be "Name". Rich text is chunked at 2000 characters and blocks
batched at 100 per request — both are hard API limits that reject the entire call.

**Not verified against real Notion:** that needs the user's integration token.

### G27 · Retention and the privacy surface
**Depends on:** G6
**Build:** Background sweeper deleting audio past `audio_expires_at` (default 7 days, configurable, "keep forever" allowed). Manual purge-all-audio. A privacy panel showing which provider generated each panel, since `panels.provider` is stored per generation. Confirm no telemetry anywhere in the build.
**Done when:** audio older than the window is gone on next launch while transcripts and notes are untouched, and the panel truthfully reports the local-vs-cloud provenance of every generation.
**Status:** ✅ Done. The sweeper runs at launch; only `audio_path` is cleared, never
the row, because the transcript is the durable record and the audio is a re-listening
aid. Three mutations fail the suite: deleting the meeting row, treating a null expiry
as expired, and ignoring the expiry altogether. A missing file counts as success — an
interrupted sweep must not leave a row pointing at nothing forever.

"No telemetry" is **checked, not claimed**: a test walks the whole source tree for
analytics hosts and SDK call shapes. It deliberately does not match the bare word
"telemetry", because the privacy panel uses that word to say there is none, and a
check that fires on its own denial is one nobody can keep passing honestly.

**A real bug caught here.** `panels.provider` stores a *display label*
("LM Studio"), and the first version of the privacy panel matched it against
snake_case enum names in TypeScript — so **every local generation was reported as
cloud**, in the one surface whose entire job is telling the truth about where data
went. Classification now happens in Rust, where the enum lives, and accepts both
forms so rows written before the fix still classify.

### G28 · Onboarding
**Depends on:** G13, G20, G23
**Build:** First-run flow: permissions → ASR model download → provider choice (including the fully-local path) → optional calendar connect → detection defaults explanation.
**Done when:** a fresh machine with no keys and no calendar reaches a first successful recording without touching a settings screen.
**Status:** ✅ Done. Every step is actionable in place — the permission prompt, the
model download and the provider choice all happen inside first run rather than
sending the user to a settings screen and hoping they come back.

Progress is **derived from what is true, not counted**: someone who revokes a
permission halfway through lands back on the step that actually blocks them, not on
step 4 because a stored number says so. It hides itself for anyone who has already
recorded something — a returning user with meetings in the library has plainly got
past setup.

### G29 · Packaging and release
**Depends on:** all
**Build:** Developer ID signing, notarization, hardened runtime with the audio/screen entitlements, DMG, Sparkle updater with an appcast. CI to build and notarize a release.
**Done when:** the DMG installs on a clean machine with no Gatekeeper warning and successfully auto-updates from the previous version.
**Status:** ⚠️ **Pipeline built; the done-when cannot be met without your Apple
Developer credentials.** `security find-identity` reports **0 valid signing
identities** on this machine, so nothing here can be signed or notarized.

What exists and was verified:

- Hardened-runtime entitlements, with a comment on each explaining why it is needed —
  microphone, library validation disabled (WhisperKit loads CoreML at runtime), and
  unsigned executable memory (`llama-server` is downloaded at runtime and is
  therefore not signed by us; without it the fully-local path dies on a machine where
  everything else works).
- **The DMG builds and was mounted and inspected**: 7.7 MB, drag-to-Applications
  layout, both binaries present. `codesign` reports `adhoc, linker-signed` and
  `spctl` rejects it — exactly what an unsigned build should do.
- A release workflow that signs and notarizes when the secrets are present and
  **warns loudly when they are not**, rather than emitting a half-signed artifact.
  It runs `codesign --verify` and `spctl --assess` after building, so the claim is
  checked by the same tool Gatekeeper consults.

**Deviation:** updates use **Tauri's own updater, not Sparkle**. It verifies a
minisign-signed manifest — the property Sparkle would have been chosen for — and is
the idiomatic path for a Tauri app rather than a second update system bolted on.

**To finish this yourself:** a Developer ID Application certificate, an app-specific
password, and `pnpm tauri signer generate` for the update key. Set
`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` and `TAURI_SIGNING_PRIVATE_KEY` as
repository secrets, put the public key in `tauri.conf.json`, and push a `v*` tag.

---

## Sequencing at a glance

```
G1 ──┬─ G2 (spike) ────────────┐
     ├─ G3 (data) ─────────┐   │
     └─ G4 (sidecar) ─ G5 ─┴─ G6 ─ G7 ─ G8 ─ G9 ─ G10 ─ G11
                                                    │      │
                          G12 ─ G13                 │      │
                            └──────────── G14 ─ G15 ┘      │
                          G16 ─────┴ G17 ─ G18             │
                                                     G19 ──┤
                          G20 ─┬─ G22 ─ G23                │
                          G21 ─┘                           │
                          G24 ─ G25 ────────────────────────┘
                          G26 · G27 · G28 ─ G29
```

**First useful build:** G8. **First build worth using daily:** G15. **First build that feels like Granola:** G22. **v1:** G29.

Phases 3 (generation) and 4 (differentiator) can overlap with Phase 5 (autonomy) if work is ever parallelised — they share only the data layer. Everything in Phases 0–2 is strictly sequential.
