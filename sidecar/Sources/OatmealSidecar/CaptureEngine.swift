import AVFoundation
import Foundation
import SidecarCore
import SidecarProtocol

/// Owns both capture streams, the pre-roll ring buffers, and the recording file.
///
/// Three states, because "is audio being captured" and "is audio being kept" are
/// genuinely different questions:
///
///   idle      nothing is captured at all
///   armed     capturing into a rolling ~60s buffer, nothing on disk
///   recording draining the pre-roll and appending to a file
///
/// Arming separately is what makes "start recording" retroactive without leaving
/// the mic hot from launch: the app arms when a meeting is detected, and the
/// pre-roll recovers whatever was already being said when the user hit record.
final class CaptureEngine {
    enum State: String {
        case idle
        case armed
        case recording
    }

    static let sampleRate: Double = 16_000
    /// How much audio "record now" can reach back for.
    static let preRollSeconds: Double = 60
    /// How often aligned audio is flushed to the file.
    private static let flushInterval: TimeInterval = 0.25

    private let lock = NSLock()
    private var state: State = .idle

    private var micCapture: MicCapture?
    private var systemCapture: SystemAudioCapture?

    private var micRing = RingBuffer(seconds: preRollSeconds, sampleRate: sampleRate)
    private var systemRing = RingBuffer(seconds: preRollSeconds, sampleRate: sampleRate)

    private var aligner = StreamAligner()
    private var recorder: AudioRecorder?
    private var flushTimer: DispatchSourceTimer?
    private let flushQueue = DispatchQueue(label: "oatmeal.sidecar.flush")

    /// Rolling levels for the recording indicator.
    private var micLevel: Double = 0
    private var systemLevel: Double = 0

    /// Called with every settled block of mic/system audio while recording, so
    /// the transcriber (G7) can consume the same stream the file receives.
    var onAudio: ((AudioSource, [Float]) -> Void)?

    var currentState: State {
        lock.lock()
        defer { lock.unlock() }
        return state
    }

    // MARK: Lifecycle

    /// Begins capture into the pre-roll buffers. Nothing reaches disk yet.
    func arm() throws {
        lock.lock()
        guard state == .idle else {
            lock.unlock()
            return
        }
        state = .armed
        lock.unlock()

        let mic = MicCapture { [weak self] samples in
            self?.ingest(.mic, samples)
        }
        let system = SystemAudioCapture { [weak self] samples in
            self?.ingest(.system, samples)
        }

        do {
            try mic.start()
        } catch {
            lock.lock()
            state = .idle
            lock.unlock()
            throw error
        }
        micCapture = mic

        // System audio is started asynchronously by ScreenCaptureKit. A failure
        // here leaves the mic running rather than aborting: half a meeting is
        // still worth capturing, and the error is reported upward.
        systemCapture = system
        Task { [weak self] in
            do {
                try await system.start()
            } catch {
                Log.error("system audio failed to start: \(error)")
                self?.onError?("System audio unavailable: \(error)")
            }
        }

        Log.info("armed (pre-roll \(Int(Self.preRollSeconds))s)")
    }

    var onError: ((String) -> Void)?

    /// Promotes an armed engine to recording, seeding the file with the pre-roll.
    func startRecording(to url: URL) throws {
        if currentState == .idle { try arm() }

        let recorder = try AudioRecorder(url: url, sampleRate: Self.sampleRate)

        lock.lock()
        // Seed from the ring buffers so the file opens with what was already
        // being said. Both rings are snapshotted under the same lock so they
        // describe the same instant.
        let micPre = micRing.snapshot()
        let systemPre = systemRing.snapshot()
        micRing.removeAll()
        systemRing.removeAll()

        aligner = StreamAligner()
        aligner.push(mic: micPre)
        aligner.push(system: systemPre)

        self.recorder = recorder
        state = .recording
        lock.unlock()

        Log.info(
            "recording -> \(url.lastPathComponent) "
                + "(pre-roll mic \(micPre.count) / system \(systemPre.count) samples)")

        startFlushing()
    }

    /// Stops recording, finalises the file, and returns where it landed.
    /// The engine stays armed so a following meeting still has its pre-roll.
    func stopRecording() -> (path: String?, durationMs: Int) {
        stopFlushing()

        lock.lock()
        let recorder = self.recorder
        self.recorder = nil
        let tail = aligner.drain()
        aligner = StreamAligner()
        if state == .recording { state = .armed }
        lock.unlock()

        guard let recorder else { return (nil, 0) }

        // Squares off the file: whichever channel was behind gets padded so both
        // end at the same length.
        if let tail {
            try? recorder.write(mic: tail.mic, system: tail.system)
        }

        // Finalise before reporting: the path we emit must point at a complete,
        // readable file, not one still missing its container trailer.
        let duration = recorder.durationMs
        recorder.close()

        return (recorder.url.path, duration)
    }

    func disarm() {
        stopFlushing()

        lock.lock()
        state = .idle
        micRing.removeAll()
        systemRing.removeAll()
        aligner = StreamAligner()
        recorder = nil
        lock.unlock()

        micCapture?.stop()
        micCapture = nil

        let system = systemCapture
        systemCapture = nil
        Task { await system?.stop() }

        Log.info("disarmed")
    }

    func levels() -> (mic: Double, system: Double) {
        lock.lock()
        defer { lock.unlock() }
        return (micLevel, systemLevel)
    }

    // MARK: Internals

    private func ingest(_ source: AudioSource, _ samples: [Float]) {
        guard !samples.isEmpty else { return }
        let level = rms(samples)

        lock.lock()
        switch source {
        case .mic:
            micLevel = level
            if state == .recording {
                aligner.push(mic: samples)
            } else {
                micRing.append(samples)
            }
        case .system:
            systemLevel = level
            if state == .recording {
                aligner.push(system: samples)
            } else {
                systemRing.append(samples)
            }
        }
        let recording = state == .recording
        lock.unlock()

        if recording { onAudio?(source, samples) }
    }

    private func rms(_ samples: [Float]) -> Double {
        guard !samples.isEmpty else { return 0 }
        let sum = samples.reduce(0.0) { $0 + Double($1 * $1) }
        return (sum / Double(samples.count)).squareRoot()
    }

    private func startFlushing() {
        let timer = DispatchSource.makeTimerSource(queue: flushQueue)
        timer.schedule(deadline: .now() + Self.flushInterval, repeating: Self.flushInterval)
        timer.setEventHandler { [weak self] in self?.flush() }
        timer.resume()
        flushTimer = timer
    }

    private func stopFlushing() {
        flushTimer?.cancel()
        flushTimer = nil
    }

    /// Writes whatever both channels have settled on.
    ///
    /// Only fully-overlapped frames are written here; a channel that is briefly
    /// behind gets to catch up on the next tick rather than being padded with
    /// silence it would later have filled. `drain()` at stop does the padding.
    private func flush() {
        lock.lock()
        guard state == .recording, let recorder else {
            lock.unlock()
            return
        }
        let frames = aligner.alignedFrames
        guard frames > 0 else {
            lock.unlock()
            return
        }
        let block = aligner.pull(frames: frames)
        lock.unlock()

        guard let block else { return }
        do {
            try recorder.write(mic: block.mic, system: block.system)
        } catch {
            Log.error("write failed: \(error)")
            onError?("Recording write failed: \(error)")
        }
    }
}
