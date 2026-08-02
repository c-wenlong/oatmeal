import Foundation
import SidecarCore
import SidecarProtocol

// Oatmeal sidecar.
//
// Reads newline-delimited JSON commands on stdin, writes newline-delimited JSON
// events on stdout. Diagnostics go to stderr so stdout stays pure protocol.
//
// Flags:
//   --fixture         emit the scripted transcript instead of capturing audio.
//                     Used by the integration tests and the dev harness so they
//                     need neither a microphone nor Screen Recording permission.
//   --fast            collapse scripted delays (fixture mode only)
//   --crash-on-start  exit(9) immediately after `start`, to exercise supervisor restart

let arguments = Set(CommandLine.arguments.dropFirst())
let fixtureMode = arguments.contains("--fixture")
let fast = arguments.contains("--fast")
let crashOnStart = arguments.contains("--crash-on-start")

let stdoutLock = NSLock()

func emit(_ event: SidecarEvent) {
    guard let line = try? WireCodec.encode(event) else { return }
    stdoutLock.lock()
    defer { stdoutLock.unlock() }
    FileHandle.standardOutput.write(Data((line + "\n").utf8))
}

/// Guards against a second `start` racing the first session's emitter.
final class SessionState {
    private let lock = NSLock()
    private var generation = 0

    func begin() -> Int {
        lock.lock()
        defer { lock.unlock() }
        generation += 1
        return generation
    }

    func isCurrent(_ token: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return generation == token
    }

    func end() {
        lock.lock()
        defer { lock.unlock() }
        generation += 1
    }
}

let session = SessionState()
let capture = CaptureEngine()

capture.onError = { message in
    emit(.error(message: message, fatal: false))
}

// MARK: - Transcription

/// `small.en` is the shipping default (SPEC section 5); override for a faster
/// or multilingual model.
let modelName = ProcessInfo.processInfo.environment["OATMEAL_ASR_MODEL"] ?? "small.en"
let transcriber = Transcriber(model: modelName)
/// Cross-channel echo guard.
///
/// Sits between the two transcribers and the wire, because it is the only point
/// that sees both channels' finals in the order they settled.
///
/// A class with the lock inside rather than a global `var` and a separate lock:
/// the two transcribers are independent tasks and emit concurrently, so the
/// synchronisation has to be part of the thing being shared. `@unchecked
/// Sendable` is the claim that `lock` makes this safe, and it is true only
/// because every access to `suppressor` goes through `admit`.
final class EchoGate: @unchecked Sendable {
    private let lock = NSLock()
    private var suppressor = EchoSuppressor()

    func admit(isMic: Bool, text: String, t0: Int, t1: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return suppressor.admit(isMic: isMic, text: text, t0: t0, t1: t1)
    }
}

let echoGate = EchoGate()

/// Emits a transcript event unless it is the speakers bleeding into the mic.
///
/// Only `final` is judged. Partials are in-flight text that a later final
/// supersedes, so suppressing them would make the live view flicker without
/// changing what is ultimately stored.
func emitTranscript(_ event: SidecarEvent) {
    switch event {
    case .final(let source, let text, let t0, let t1, _):
        if !echoGate.admit(isMic: source == .mic, text: text, t0: t0, t1: t1) {
            // Logged rather than silent: a suppressor that is too eager deletes
            // the user's own words, and there would otherwise be no trace of it
            // having happened.
            Log.info("Dropped mic line as speaker bleed: \(text)")
            return
        }
        emit(event)
    default:
        emit(event)
    }
}

let micTranscriber = StreamingTranscriber(
    source: .mic, transcriber: transcriber, emit: emitTranscript)
let systemTranscriber = StreamingTranscriber(
    source: .system, transcriber: transcriber, emit: emitTranscript)

// Audio only reaches the model while recording; arming alone never transcribes.
capture.onAudio = { source, samples in
    Task {
        switch source {
        case .mic: await micTranscriber.append(samples)
        case .system: await systemTranscriber.append(samples)
        }
    }
}

/// Loads the model once, off the command loop so the handshake isn't delayed by
/// a multi-hundred-megabyte download.
func ensureModelLoaded() {
    Task {
        do {
            try await transcriber.load(onEvent: emit)
        } catch {
            emit(
                .error(
                    message: "ASR model unavailable: \(error)", fatal: false))
        }
    }
}

// MARK: - Fixture mode

func runScript(token: Int) {
    for step in ScriptedSession.transcript {
        guard session.isCurrent(token) else { return }
        Thread.sleep(forTimeInterval: Double(fast ? 1 : step.delayMs) / 1000.0)
        guard session.isCurrent(token) else { return }
        emit(step.event)
    }
}

// MARK: - Real capture

/// Where recordings land. Kept beside the database so retention (G27) has one
/// directory to sweep.
func recordingsDirectory() throws -> URL {
    let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    let dir = base.appendingPathComponent("com.kaichen.oatmeal/recordings", isDirectory: true)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return dir
}

func recordingURL(meetingId: String) throws -> URL {
    // Timestamped so a re-recorded meeting never silently overwrites its predecessor.
    let stamp = Int(Date().timeIntervalSince1970)
    let safeId = meetingId.replacingOccurrences(
        of: "[^A-Za-z0-9_.-]", with: "-", options: .regularExpression)
    return try recordingsDirectory()
        .appendingPathComponent("\(safeId)-\(stamp).m4a")
}

/// Emits input levels while armed or recording, for the UI meter.
var levelTimer: DispatchSourceTimer?

func startLevelReporting() {
    guard levelTimer == nil else { return }
    let timer = DispatchSource.makeTimerSource(
        queue: DispatchQueue(label: "oatmeal.sidecar.levels"))
    timer.schedule(deadline: .now() + 0.2, repeating: 0.2)
    timer.setEventHandler {
        let levels = capture.levels()
        emit(.level(mic: levels.mic, system: levels.system))
    }
    timer.resume()
    levelTimer = timer
}

func stopLevelReporting() {
    levelTimer?.cancel()
    levelTimer = nil
}

// stdout is a pipe here, which makes it fully buffered by default; without this
// the parent sees nothing until the buffer fills.
setvbuf(stdout, nil, _IONBF, 0)

emit(.ready(version: "0.1.0", protocolVersion: PROTOCOL_VERSION))
Log.info("ready (fixture=\(fixtureMode) fast=\(fast))")

while let line = readLine(strippingNewline: true) {
    if line.trimmingCharacters(in: .whitespaces).isEmpty { continue }

    let command: SidecarCommand
    do {
        command = try WireCodec.decodeCommand(line)
    } catch {
        // A bad line is the caller's bug, not a reason to die — report and keep
        // serving, so one malformed command can't take down a recording.
        emit(.error(message: "could not parse command: \(error)", fatal: false))
        continue
    }

    switch command {
    case .arm:
        guard !fixtureMode else {
            emit(.error(message: "arm is unavailable in fixture mode", fatal: false))
            break
        }
        do {
            try capture.arm()
            startLevelReporting()
            ensureModelLoaded()
        } catch {
            emit(.error(message: "could not arm capture: \(error)", fatal: false))
        }

    case .disarm:
        stopLevelReporting()
        capture.disarm()

    case let .start(meetingId, sources):
        Log.info("start meeting=\(meetingId) sources=\(sources.map(\.rawValue))")
        if crashOnStart {
            Log.info("--crash-on-start: exiting 9")
            exit(9)
        }

        if fixtureMode {
            let token = session.begin()
            Thread.detachNewThread { runScript(token: token) }
            break
        }

        do {
            let url = try recordingURL(meetingId: meetingId)
            Task {
                await micTranscriber.reset()
                await systemTranscriber.reset()
            }
            try capture.startRecording(to: url)
            startLevelReporting()
            ensureModelLoaded()
        } catch {
            emit(.error(message: "could not start recording: \(error)", fatal: false))
        }

    case .stop:
        if fixtureMode {
            session.end()
            emit(.stopped(audioPath: nil, durationMs: ScriptedSession.durationMs))
            Log.info("stopped (fixture)")
            break
        }

        let result = capture.stopRecording()
        // Flush before announcing the stop, so the last thing said makes it
        // into the transcript rather than being dropped with the buffer.
        let flushed = DispatchSemaphore(value: 0)
        Task {
            await micTranscriber.flush()
            await systemTranscriber.flush()
            flushed.signal()
        }
        _ = flushed.wait(timeout: .now() + 20)
        emit(.stopped(audioPath: result.path, durationMs: result.durationMs))
        Log.info("stopped -> \(result.path ?? "no file") (\(result.durationMs)ms)")

    case .ping:
        emit(.pong)

    case let .permissions(request):
        // TCC calls can block on a system prompt, so never run them on the
        // command loop — a pending dialog would stall every later command.
        Thread.detachNewThread {
            let semaphore = DispatchSemaphore(value: 0)
            Task {
                let event = request ? await Permissions.request() : await Permissions.snapshot()
                emit(event)
                semaphore.signal()
            }
            semaphore.wait()
        }
    }
}

// stdin closed: the parent went away, so we should too.
stopLevelReporting()
capture.disarm()
Log.info("stdin closed, exiting")
exit(0)
