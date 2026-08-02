# Oatmeal — v1 Spec

**Status:** draft for review. Not approved to build.
**Companion doc:** [granola-research.md](./granola-research.md)

> ⚠️ **One open input.** Your description of Granola's actual look/feel was cut off mid-message. §9 (UI surfaces) is written from research, not from your experience of the product. Expect to revise it once you paste that description — nothing else in this spec depends on it.

---

## 1. Thesis

Three things make Oatmeal different from Ghost Pepper and from every bot-based notetaker:

1. **Notes are the anchor, not an afterthought.** You type sparse notes *during* the meeting. The summarizer receives notes + transcript together, and your notes tell it what mattered.
2. **Provenance is visible.** Every note block links back to the transcript spans it came from, and every generated bullet cites its sources. You can always answer "why does it say that?"
3. **It shows up on its own.** Calendar-driven popup before the meeting starts, plus mic-activation detection for ad-hoc calls — with a per-app allowlist so dictation tools don't false-trigger.

Everything runs on the Mac. The only thing that may leave the machine is a summarization request, and only if you point it at a cloud provider.

## 2. Decisions (locked)

| Area | Decision |
|---|---|
| Shell | **Tauri v2** + React + TypeScript |
| Core | Rust (Tauri backend) |
| Native | Swift sidecar for audio capture + ASR |
| Platform | macOS only, Apple Silicon |
| Storage | **Local SQLite**, no cloud, no account |
| ASR | **WhisperKit** in the Swift sidecar, streaming |
| LLM | **Local + BYOK.** Anthropic, OpenAI, OpenRouter, Ollama/LM Studio, bundled `llama-server` |
| Audio retention | **Keep, auto-expire** (default 7 days, configurable) |
| Detection | Calendar event + mic activation (per-app filtered) |
| Note→transcript linking | **Layered**: temporal → semantic → LLM citation |
| Connectors | Calendar (Google/Outlook) in, Notion export out |
| v1 features | Templates, Folders + search, AI chat over meetings, Notion export |

### Assumptions I'm making (flag if wrong)

- **Manual start is included** even though you didn't select it. A global hotkey + menu-bar "Record now" is needed for in-person meetings and for when detection misses. Cheap to build, dangerous to omit.
- Browser-tab/meeting-app detection is **out of v1** (you didn't select it). Mic activation covers most of the same ground, just slightly later.
- No team/sharing features. Single user, single machine.

### Non-goals for v1

Windows/Linux. Mobile. Any hosted backend or account system. Real-time translation. Speaker identification by name (we do source attribution only — see §5). Slack/CRM connectors.

---

## 3. Architecture

```
┌─────────────────────────────────────────────────────────┐
│  React + TypeScript (Tauri webview)                     │
│  Notepad · Live transcript · Library · Chat · Settings  │
└───────────────────────┬─────────────────────────────────┘
                        │ Tauri IPC (commands + events)
┌───────────────────────┴─────────────────────────────────┐
│  Rust core                                              │
│  ├─ meeting orchestrator (state machine)                │
│  ├─ SQLite (rusqlite) + FTS5 + sqlite-vec               │
│  ├─ linker (temporal / semantic scoring)                │
│  ├─ LLM provider layer (OpenAI-shaped, +Anthropic)      │
│  ├─ calendar sync (Google / Microsoft Graph)            │
│  ├─ Notion export                                       │
│  └─ retention sweeper                                   │
└──────┬──────────────────────────────┬───────────────────┘
       │ JSON lines over stdio        │ spawn (JIT-downloaded)
┌──────┴────────────────────┐  ┌──────┴────────────────────┐
│  Swift sidecar            │  │  llama-server (optional)  │
│  ├─ ScreenCaptureKit      │  │  OpenAI-compatible :8080  │
│  ├─ AVFoundation (mic)    │  └───────────────────────────┘
│  ├─ WhisperKit streaming  │
│  └─ mic-activation watch  │
└───────────────────────────┘
```

**Why a Swift sidecar rather than pure Rust:** ScreenCaptureKit and WhisperKit are Objective-C/Swift APIs with no good Rust bindings, and WhisperKit's CoreML path is the reason we get Neural Engine acceleration. The sidecar speaks newline-delimited JSON over stdio — audio bytes never cross the boundary, only transcript events. That keeps the IPC cheap and the sidecar independently testable.

**Sidecar protocol (sketch):**

```
→ {"cmd":"start","meeting_id":"...","sources":["mic","system"]}
← {"ev":"partial","source":"system","text":"so the deadline is","t0":12400,"t1":13900}
← {"ev":"final","source":"system","text":"So the deadline is the 14th.","t0":12400,"t1":14700,"conf":0.93}
← {"ev":"level","mic":0.12,"system":0.44}
→ {"cmd":"stop"}
← {"ev":"stopped","audio_path":"...","duration_ms":2841000}
```

---

## 4. Audio capture

- **System audio:** ScreenCaptureKit in audio-only mode. Requires Screen Recording permission (yes, even with no video — unavoidable).
- **Mic:** AVFoundation / AVAudioEngine.
- **The two streams stay separate all the way through ASR.** This is the single highest-leverage design choice: `source: mic` is you, `source: system` is everyone else. That's free speaker attribution with no diarization model, and it's exactly what Granola's transcript schema does.
- Persisted as a single AAC/Opus file with two channels (~30MB/hr), deleted by the retention sweeper.
- Ring buffer holds the last ~60s continuously, so "start recording" retroactively captures the moment *before* you hit the button.

**Risk:** Core Audio taps (macOS 14.4+) would avoid the Screen Recording prompt and look better privacy-wise, but they're widely reported as unreliable in production and can't capture mic. Sticking with ScreenCaptureKit; revisit if the permission prompt proves to be a real adoption problem.

## 5. Transcription

WhisperKit in the sidecar, one model instance, two streams processed in interleaved chunks.

- Sliding window ~30s with ~5s overlap; overlap reconciliation to avoid duplicated words at boundaries.
- Emits `partial` events for the live UI and `final` events once a window settles. Only finals are persisted.
- Default model: `small.en` (~466MB). Selectable: `tiny.en`, `small` multilingual. Downloaded on first run from Hugging Face, cached.
- Speaker labels are `You` / `Them` from stream source. **In-person meetings degrade** — everyone lands on the mic channel as one speaker. Accepted for v1; real diarization (FluidAudio) is the fix, deferred.

## 6. Meeting detection

State machine with three trigger sources feeding one "candidate meeting" queue.

**A. Calendar.** Google Calendar + Microsoft Graph OAuth, read-only. Poll every ~5 min, cache locally. An event is meeting-shaped if it has a conferencing URL (Zoom/Meet/Teams), or ≥2 attendees, or an explicit location. Fires a popup at `event.start - lead`, `lead` configurable, default 90s.

**B. Mic activation, per-app filtered.** Poll which processes hold the audio input device. **Requires an explicit per-app rule before it ever fires:**

- Ships with a built-in allowlist: Zoom, Google Meet (Chrome/Safari/Arc), Teams, Slack, Discord, FaceTime, Webex.
- Everything else is **ignored by default.** First time an unknown app grabs the mic, we show a one-time, dismissible "Should Oatmeal offer to record when *Whisperflow* uses the mic?" with Always / Never. Never is remembered permanently.
- Settings has the full list, both columns, editable.

This directly addresses the Whisperflow case: a dictation tool never triggers a popup unless you explicitly opt it in.

**C. Manual.** Global hotkey + menu-bar item. Always available.

**Popup behavior:** a small floating window — meeting title (from calendar if known), Start / Ignore / Ignore-this-app. Never auto-records without consent. Auto-dismisses after ~60s as Ignore.

## 7. Notes ↔ transcript linking

The differentiator. Three layers, each stored with its method and score so we can debug and tune.

**Layer 0 — capture timing.** The notepad is block-structured. Each block records `first_typed_at_ms` and `last_edited_at_ms` relative to meeting start.

**Layer 1 — temporal candidates.** For a block first typed at `T`, candidate utterances fall in `[T - 45s, T + 10s]`. The asymmetry is deliberate: you type *after* hearing something. Score decays with distance from `T`.

**Layer 2 — semantic rerank.** A local embedding model embeds each note block and each candidate utterance; cosine similarity reranks. Combined score `α·temporal + β·semantic`, α and β tunable in a debug panel.
Additionally a **global** semantic pass over the whole transcript catches notes typed long after the fact — if the best global match beats the best windowed match by a margin, it's added as a second link.

**Layer 3 — LLM citations.** The summarizer receives the transcript with stable utterance IDs and the notes with block IDs, and returns structured output where every generated bullet carries `source_utterances: [id]` and optionally `from_note: block_id`. **IDs are validated against the DB and silently dropped if invalid** — this is the anti-hallucination gate.

**Surfacing it:**
- Hovering a note block highlights its linked transcript spans; hovering a transcript span highlights the notes that drew from it.
- Summary bullets carry a citation chip; clicking scrolls the transcript to that utterance.
- Note-derived bullets are visually distinguished from transcript-only bullets — so you can see at a glance what the AI added versus what you flagged.

## 8. Data model (SQLite)

```sql
folders(id, name, parent_id, created_at)

meetings(id, title, folder_id, started_at, ended_at, status,
         calendar_event_id, trigger_source,       -- calendar | mic | manual
         audio_path, audio_expires_at)

calendar_events(id, provider, external_id, title, starts_at, ends_at,
                conferencing_url, attendees_json, synced_at)

utterances(id, meeting_id, seq, source,           -- mic | system
           text, start_ms, end_ms, confidence)

note_blocks(id, meeting_id, seq, text,
            first_typed_at_ms, last_edited_at_ms)

note_links(id, note_block_id, utterance_id, method, score)
                                                  -- temporal | semantic | llm

templates(id, name, prompt, output_schema_json, is_builtin)

panels(id, meeting_id, template_id, content_json, -- regenerable output
       provider, model, generated_at)

panel_citations(id, panel_id, block_path, utterance_id, note_block_id)

embeddings(owner_type, owner_id, vector)          -- via sqlite-vec

providers(id, kind, base_url, model, keychain_ref)
detection_rules(id, bundle_id, mode)              -- allow | ignore
```

Plus FTS5 virtual tables over `utterances.text`, `note_blocks.text`, and panel plaintext.

**The key structural idea, borrowed from Granola:** a meeting holds *one immutable transcript* and *N regenerable panels*. Switching templates or swapping models regenerates a panel; it never touches the transcript or your notes. Never bake the summary into the note.

API keys live in the **macOS Keychain**, never in SQLite.

## 9. UI surfaces

> Written from research. Revise against your description.

1. **Floating pre-meeting popup** — small, title, Start / Ignore / Ignore-this-app.
2. **Meeting view** — the main screen. Notepad occupies the primary column; live transcript in a secondary panel that can be collapsed. Recording indicator + elapsed time. The notepad is the focus; the transcript is reference.
3. **Post-meeting view** — same document, now with the generated panel on top, template switcher, regenerate button, citation chips.
4. **Library** — folder tree, meeting list, search bar spanning full-text and semantic.
5. **Chat** — scoped to one meeting or a whole folder, retrieval over `utterances` + `panels`, answers cite meetings and timestamps.
6. **Settings** — models, providers/keys, detection rules (the two-column app list), calendar accounts, retention, Notion.
7. **Menu bar** — status, record now, recent meetings.

## 10. LLM provider layer

One internal interface, OpenAI chat-completions shape. Presets:

| Preset | Endpoint | Key |
|---|---|---|
| Anthropic | `api.anthropic.com` (thin adapter) | Keychain |
| OpenAI | `api.openai.com/v1` | Keychain |
| OpenRouter | `openrouter.ai/api/v1` | Keychain |
| Ollama | `localhost:11434/v1` | none |
| LM Studio | `localhost:1234/v1` | none |
| Bundled | `localhost:8080/v1` (`llama-server`) | none |

`llama-server` is **JIT-downloaded** from llama.cpp GitHub releases on first use rather than static-bundled — keeps the installer small and decouples llama.cpp updates from app releases. Adding LiteRT-LM later is a preset row *if* it exposes an OpenAI-compatible endpoint; unverified, so not promised.

Structured output (for citations) via JSON schema where supported, with a repair-retry path for local models that ignore the schema.

## 11. Privacy & permissions

| Permission | Why | When asked |
|---|---|---|
| Screen Recording | ScreenCaptureKit system audio | First recording |
| Microphone | Your voice | First recording |
| Calendar (OAuth) | Detection + attendees | Onboarding, skippable |

- Fully local by default. A cloud call happens **only** for summarization/chat, **only** with a cloud provider selected, and the UI shows which provider handled each panel (`panels.provider` is stored per generation).
- Audio auto-expires (default 7 days). Manual purge in Settings. Transcript and notes persist indefinitely.
- No telemetry.

## 12. Milestones

| # | Deliverable | Proves |
|---|---|---|
| M0 | Tauri skeleton, SQLite + migrations, Swift sidecar handshake | Plumbing |
| M1 | Dual-stream capture → WhisperKit → live transcript on screen | The hard part works |
| M2 | Notepad with per-block timing; meeting persists and reopens | Data model holds |
| M3 | Provider layer + summarization + templates + panels | Output is good |
| M4 | Layered linking + citation UI | **The differentiator** |
| M5 | Calendar sync + popup + per-app mic rules | It shows up on its own |
| M6 | Library, FTS + vector search, chat | Corpus is useful |
| M7 | Notion export, retention sweeper, onboarding, signing/notarization, DMG | Shippable |

M1 and M4 carry the real risk. M1 because dual-stream ScreenCaptureKit + streaming ASR is where this kind of project usually stalls; M4 because link quality is a tuning problem, not a coding problem, and needs real meetings to evaluate.

## 13. Open questions

1. **Your Granola description** — blocks §9.
2. **Notion export shape** — one page per meeting in a database, or append to an existing page? Which properties map to what?
3. **Template authoring** — plain prompt text, or prompt + enforced output schema? The latter makes citations far more reliable.
4. **In-person meetings** — accept single-speaker degradation in v1, or pull FluidAudio in early for diarization?
5. **Editing** — should you be able to edit the generated panel, and if so, does regenerating overwrite your edits or fork?
