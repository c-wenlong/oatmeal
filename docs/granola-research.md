# Granola: how it's built, and what we should copy

Research notes for Oatmeal. Sources listed at the bottom.

## 1. The core product insight

Granola is **not** a transcription app with a summary bolted on. The differentiator is:

> You type sparse notes during the meeting. The AI uses *your notes as anchors* into the transcript, then expands them into a structured document.

This matters architecturally. The summarizer prompt takes two inputs — raw notes + full transcript — and the notes act as an attention/relevance signal telling the model what *you* cared about. Everyone else (Otter, Fathom, Fireflies) summarizes the transcript alone, which is why their output feels generic. **This is the single feature to get right.**

Second insight: **no bot joins the call.** Granola runs as a desktop agent capturing device audio locally. Invisible to other participants, works in any app (Zoom, Meet, Teams, Slack huddles, phone on speaker, in-person). No "Fireflies.ai Notetaker has joined" awkwardness.

## 2. Tech stack (as far as it's publicly known / reverse-engineered)

| Layer | Granola's choice |
|---|---|
| Desktop shell | Electron (macOS-first; Windows came much later) |
| System audio | Apple `ScreenCaptureKit` (audio-only mode) and/or Core Audio process taps |
| Mic audio | `AVFoundation` |
| Doc model | ProseMirror JSON tree (notes + generated "panels") |
| Auth | WorkOS OAuth2, refresh-token rotation, one-time-use tokens |
| Backend | Split into themed sub-services (`berry`, `chia`, `cinnamon`, `maple`, `pecan`) |
| LLM | Cloud (GPT-4 class + reasoning models) — **no local option** |

### Data model (from the reverse-engineered API)

```
Workspace ──< DocumentList (folder) ──< Document
                                          ├── Panel[]      (ProseMirror content: raw notes, generated summary per template)
                                          └── Transcript    (utterance[]: {source: mic|system, text, ts, confidence})
```

Key endpoints they expose: `/v2/get-documents`, `/v1/get-documents-batch` (needed for *shared* docs — the v2 list endpoint omits them), `/v1/get-document-transcript`, `/v1/get-workspaces`, `/v2/get-document-lists`.

Worth stealing: **the panel abstraction.** One meeting document holds N generated views (default summary, sales-call template, 1:1 template, action items), each regenerable independently, each stored alongside the immutable transcript. Don't bake the summary into the note.

## 3. Meeting auto-detection

Granola layers several signals rather than relying on one:

1. **Calendar sync** (Google/Outlook) — the primary signal. An event with a Zoom/Meet/Teams conferencing URL in it is a strong "this is a meeting" indicator, and gives you title, attendees, and agenda for free.
2. **Mic activation watching** — an OS-level check for which processes hold the audio input device. Catches ad-hoc calls with no calendar event.
3. **Browser tab URL / app frontmost** — `meet.google.com/*`, `*.zoom.us/j/*`, `teams.microsoft.com/*`.

On detection it doesn't auto-record silently — it pops a notification/floating window asking to start. That consent step is both a privacy posture and a UX one.

## 4. Feature surface (2026)

- Notepad + live transcript side by side; summary generated on meeting end
- **Templates** — user-defined and shared-across-team output formats
- **Folders** — group meetings by client/project/deal
- **AI chat** — across a single meeting *and* across a whole folder ("what did we promise Acme in the last 5 calls?")
- **Integrations** — Slack (auto-post summary to a channel), Notion, HubSpot, Attio, Affinity, Zapier
- **MCP server** (added Feb 2026) — exposes your meeting corpus to Claude/ChatGPT. Cheap for us to build and a huge unlock.
- Audio is deleted immediately after transcription; only text is retained
- Pricing: free (25 notes, 14-day history) / $14 Business / $35 Enterprise

## 5. macOS system-audio capture: the actual decision

| Approach | Min OS | Perm needed | Notes |
|---|---|---|---|
| **ScreenCaptureKit** (audio-only) | 13.0 | Screen Recording | Modern default. Works even though we capture no video. Ghost Pepper uses this. |
| **Core Audio process taps** | 14.4 | Audio capture (lighter prompt) | Per-process audio, no screen-recording prompt — better privacy optics. But widely reported as finicky in production, and **outgoing audio only** (no mic). |
| BlackHole / virtual driver | any | user installs kext-ish driver | 6-step manual setup, breaks output routing. Non-starter for consumer UX. |

Realistic plan: **ScreenCaptureKit for system audio + AVFoundation for mic, two separate streams kept separate.** Keeping them separate gives you free speaker attribution (`source: mic` = you, `source: system` = everyone else) without a real diarization model — which is exactly what Granola's transcript schema does. Add proper diarization later only for in-person meetings where everyone is on one mic.

## 6. What Ghost Pepper already solves (in `../ghost-pepper`)

Swift/SwiftUI macOS app, macOS 14+, Apple Silicon, sandboxed. Already has: WhisperKit (Whisper tiny/small), FluidAudio (Parakeet v3, 25 langs), Qwen3-ASR, LLM.swift for local Qwen 0.8B/2B/4B summarization, ScreenCaptureKit + AVAudioEngine capture, Sparkle updates, meeting → markdown storage. It even has `Calendar/`, `SpeakerIdentity/`, and `Indexing/` modules.

So the *audio and local-model plumbing is a solved problem* and is MIT-licensed. Its gaps — matching your complaint — are the UI (hotkey-dictation-first, meetings feel secondary) and connectors (only Zo chat, Trello, and a Granola *importer*).

**The strategic question this raises is question 1 below.**

---

### Sources

- [Granola — how transcription works](https://docs.granola.ai/help-center/taking-notes/transcription)
- [Granola — AI meeting transcription: how it works](https://www.granola.ai/blog/ai-meeting-transcription-how-it-works-and-which-tools-lead-in-2026)
- [Granola — recording Zoom/Teams/Meet/in-person](https://www.granola.ai/blog/how-to-record-a-meeting-and-have-ai-summarize-it-zoom-teams-google-meet-and-in-person)
- [getprobo/reverse-engineering-granola-api](https://github.com/getprobo/reverse-engineering-granola-api)
- [Reverse-engineering Granola's data export](https://medium.com/@danielmoon_65473/reverse-engineering-granolas-data-export-with-claude-code-and-a-script-to-do-it-d3d292452a43)
- [Recall.ai — how to get access to system audio on macOS](https://www.recall.ai/blog/how-to-get-access-to-system-audio)
- [Recall.ai — Core Audio Taps deep dive](https://www.recall.ai/blog/core-audio-taps)
- [Building a 100% local meeting transcription app with whisper.cpp + ScreenCaptureKit](https://dev.to/thehwang/building-a-100-local-meeting-transcription-app-for-macos-with-whispercpp-and-screencapturekit-33m7)
- [Granola review 2026 — features & pricing](https://www.bluedothq.com/blog/granola-review)
- [Granola AI meeting notes 2026 walkthrough](https://aiproductivity.ai/guides/granola-ai-meeting-notes-guide/)
