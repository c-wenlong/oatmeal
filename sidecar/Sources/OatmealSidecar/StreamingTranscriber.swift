import Foundation
import SidecarCore
import SidecarProtocol

/// Turns one capture stream into `partial` and `final` events.
///
/// One instance per source, so `mic` and `system` are transcribed and settled
/// independently — that separation is what preserves speaker attribution
/// without a diarization model.
actor StreamingTranscriber {
    private let source: AudioSource
    private let policy: WindowPolicy
    private let vad: VoiceActivityDetector
    private let transcriber: Transcriber
    private let emit: @Sendable (SidecarEvent) -> Void

    /// Audio not yet settled into a `final`.
    private var buffer: [Float] = []
    /// Samples already settled, so timestamps stay absolute across windows.
    private var settledSamples: Int = 0
    /// Text emitted so far for the current window, used to suppress the overlap.
    private var settledText: String = ""
    private var lastPartialAt: Date = .distantPast
    /// Guards against two model runs for the same stream overlapping.
    private var running = false

    init(
        source: AudioSource,
        transcriber: Transcriber,
        policy: WindowPolicy = WindowPolicy(),
        vad: VoiceActivityDetector = VoiceActivityDetector(),
        emit: @escaping @Sendable (SidecarEvent) -> Void
    ) {
        self.source = source
        self.transcriber = transcriber
        self.policy = policy
        self.vad = vad
        self.emit = emit
    }

    func reset() {
        buffer.removeAll()
        settledSamples = 0
        settledText = ""
        lastPartialAt = .distantPast
    }

    func append(_ samples: [Float]) async {
        buffer.append(contentsOf: samples)
        await pump()
    }

    /// Transcribes whatever is left, at stop.
    func flush() async {
        guard policy.shouldFlush(bufferedSamples: buffer.count) else { return }
        await settle()
    }

    private func pump() async {
        guard !running else { return }

        let elapsed = Date().timeIntervalSince(lastPartialAt)
        switch policy.next(bufferedSamples: buffer.count, secondsSinceLastPartial: elapsed) {
        case .wait:
            return
        case .partial:
            await emitPartial()
        case .settle:
            await settle()
        }
    }

    private func emitPartial() async {
        // The gate matters most here: partials run every couple of seconds, so
        // an ungated silent stream would ask the model to invent text ~30 times
        // a minute (G2 finding).
        guard vad.containsSpeech(buffer) else {
            lastPartialAt = Date()
            return
        }

        running = true
        defer { running = false }
        lastPartialAt = Date()

        guard let text = await transcriber.transcribe(buffer) else { return }
        guard let addition = TranscriptMerger.newText(previous: settledText, current: text) else {
            return
        }

        emit(
            .partial(
                source: source,
                text: addition,
                t0: msFor(sampleOffset: settledSamples),
                t1: msFor(sampleOffset: settledSamples + buffer.count)))
    }

    private func settle() async {
        guard vad.containsSpeech(buffer) else {
            // Nothing was said in this window. Drop it entirely rather than
            // asking the model, and advance the clock so timestamps stay true.
            settledSamples += max(0, buffer.count - policy.retainedSamples())
            trimToOverlap()
            return
        }

        running = true
        defer { running = false }

        let windowStart = settledSamples
        let windowSamples = buffer.count

        if let text = await transcriber.transcribe(buffer),
            let addition = TranscriptMerger.newText(previous: settledText, current: text)
        {
            emit(
                .final(
                    source: source,
                    text: addition,
                    t0: msFor(sampleOffset: windowStart),
                    t1: msFor(sampleOffset: windowStart + windowSamples),
                    conf: nil))
            settledText = text
        }

        settledSamples += max(0, windowSamples - policy.retainedSamples())
        trimToOverlap()
        lastPartialAt = Date()
    }

    /// Keeps the overlap tail so a word straddling the boundary is seen whole by
    /// the next window.
    private func trimToOverlap() {
        let keep = policy.retainedSamples()
        if buffer.count > keep {
            buffer.removeFirst(buffer.count - keep)
        }
    }

    private func msFor(sampleOffset: Int) -> Int {
        Int((Double(sampleOffset) / policy.sampleRate) * 1000.0)
    }
}
