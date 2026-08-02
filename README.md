# Oatmeal

[![CI](https://github.com/c-wenlong/oatmeal/actions/workflows/ci.yml/badge.svg)](https://github.com/c-wenlong/oatmeal/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/c-wenlong/oatmeal/branch/main/graph/badge.svg)](https://codecov.io/gh/c-wenlong/oatmeal)

Local-first AI meeting notepad for macOS. Autodetects meetings, captures system + mic audio, transcribes on device, and turns your sparse in-meeting notes into a structured summary — with every generated claim traceable back to the transcript span it came from.

Nothing leaves your Mac unless you point the summarizer at a cloud provider.

> **Status: Phase 0 complete.** Foundations only — the app is a self-test harness,
> not yet a notepad. The transcript it shows is a scripted fixture; real audio
> arrives in G6/G7. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Documentation

| Doc | What it is |
|---|---|
| [docs/SPEC.md](docs/SPEC.md) | **Source of truth.** Architecture, data model, decisions. |
| [docs/ROADMAP.md](docs/ROADMAP.md) | 29 sequential goals to v1, each with an observable "Done when". |
| [docs/granola-research.md](docs/granola-research.md) | How Granola is built, and what we borrow. |

## Requirements

- macOS 14+ (developed on 26.x), Apple Silicon
- Node 22+, pnpm 10+
- Rust 1.86+
- Xcode 26+ / Swift 6.2+

## Layout

```
src/          React + TypeScript frontend
src-tauri/    Rust core — SQLite, sidecar supervision, Tauri commands
sidecar/      Swift sidecar
  SidecarProtocol/  wire format shared with Rust
  SidecarCore/      pure audio logic (ring buffer, alignment, VAD, filters)
  OatmealSidecar/   ScreenCaptureKit, AVFoundation, WhisperKit
docs/         Spec, roadmap, research, audio findings
```

The Swift sidecar exists because ScreenCaptureKit and WhisperKit have no good Rust bindings, and WhisperKit's CoreML path is what buys us Neural Engine acceleration. Audio bytes never cross the process boundary — only transcript events, as newline-delimited JSON over stdio.

## Development

```bash
pnpm install
pnpm sidecar:build     # build the Swift sidecar into src-tauri/binaries/
pnpm tauri dev
```

The app opens a build harness. Each card proves one piece works end to end
rather than merely compiling:

| Card | Proves |
|---|---|
| **Record** | Captures both streams, transcribes on device, persists attributed lines to SQLite, and reopens them after a restart |
| **Permissions** | Microphone and Screen Recording state, with deep links and the stale-grant relaunch case |
| **Rust core** | Tauri IPC round-trips between the webview and Rust |
| **Sidecar** | The Swift sidecar spawns, handshakes, streams events, and is restarted when it dies |
| **Data layer** | Migrations, FTS5 Porter stemming, and sqlite-vec nearest-neighbour all work *on this machine* — against a scratch in-memory DB, so it never touches real meetings |

**Recording has three states.** `Arm` starts capture into a rolling ~60s pre-roll
without writing anything to disk; `Start recording` opens a file seeded with that
pre-roll, so it captures the sentence someone was midway through. Nothing is
listened to until you arm.

`OATMEAL_ASR_MODEL` selects the speech model (`small.en` default, `tiny.en` for
fast iteration). `OATMEAL_SIDECAR_FIXTURE=1` swaps real capture for a scripted
transcript, useful on a machine without permissions.

To watch the supervisor recover from a crash, click **Simulate crash** — or kill it
from outside and watch the log report a new pid on attempt 2:

```bash
kill -9 $(pgrep -f oatmeal-sidecar)
```

Two dev-only env vars drive the app without clicking, because macOS withholds
Accessibility permission from most automation: `OATMEAL_HARNESS_AUTOSTART=1`
spawns and arms the sidecar at launch, and `OATMEAL_HARNESS_RECORD_SECONDS=N`
additionally runs one full N-second recording through the normal code path.
Neither does anything unless set.

## Tests

```bash
pnpm verify            # typecheck + lint + frontend tests (68)
pnpm sidecar:test      # Swift protocol + audio-core tests (84)
cd src-tauri && cargo test    # Rust unit + integration (62)
```

Coverage, as CI reports it:

```bash
pnpm test:coverage     # -> coverage/frontend/lcov.info
pnpm sidecar:coverage  # -> coverage/swift-lcov.info
cd src-tauri && cargo llvm-cov --lcov --output-path ../coverage/rust-lcov.info
```

[CI](.github/workflows/ci.yml) runs all three on every push and PR, uploading each
under its own Codecov flag so a drop is attributable to the layer that caused it.
The frontend job runs on Linux; only the jobs that genuinely need Apple toolchains
pay for a macOS runner.

The Rust integration tests in `src-tauri/tests/` drive the **real** Swift binary —
handshake, session, external kill and restart, and crash-loop abandonment. They
skip rather than fail when the binary hasn't been built, so run `pnpm sidecar:build`
first to get full coverage.

## License

MIT
