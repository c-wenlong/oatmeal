import Foundation

/// Decides when to transcribe and how much of the buffer to keep.
///
/// Two competing pressures: partials must appear fast enough to feel live
/// (~2s), while finals need a long enough window for Whisper to be accurate and
/// enough overlap that a word straddling a boundary is seen whole by one side.
///
/// Pure so the timing rules are testable without a model, a microphone, or
/// waiting 30 seconds per assertion.
public struct WindowPolicy {
    /// Longest audio handed to the model in one go.
    public let windowSeconds: Double
    /// Retained after a final, so the next window sees the tail of this one.
    public let overlapSeconds: Double
    /// How often in-flight text is refreshed.
    public let partialIntervalSeconds: Double
    public let sampleRate: Double

    public init(
        windowSeconds: Double = 30,
        overlapSeconds: Double = 5,
        partialIntervalSeconds: Double = 2,
        sampleRate: Double = 16_000
    ) {
        precondition(overlapSeconds < windowSeconds, "overlap must fit inside the window")
        self.windowSeconds = windowSeconds
        self.overlapSeconds = overlapSeconds
        self.partialIntervalSeconds = partialIntervalSeconds
        self.sampleRate = sampleRate
    }

    public enum Action: Equatable {
        /// Not enough new audio to be worth a model run.
        case wait
        /// Transcribe the buffer for in-flight display; keep everything.
        case partial
        /// Transcribe and settle; keep only the overlap tail.
        case settle
    }

    public func seconds(forSamples count: Int) -> Double {
        Double(count) / sampleRate
    }

    /// Samples to retain after a settle.
    public func retainedSamples() -> Int {
        Int(overlapSeconds * sampleRate)
    }

    /// What to do with a buffer of `bufferedSamples`, given how long since the
    /// last partial was emitted.
    ///
    /// Settling wins over partials: once the window is full there is no point
    /// emitting in-flight text for audio we are about to finalise anyway.
    public func next(bufferedSamples: Int, secondsSinceLastPartial: Double) -> Action {
        let buffered = seconds(forSamples: bufferedSamples)
        if buffered >= windowSeconds { return .settle }
        if buffered > 0 && secondsSinceLastPartial >= partialIntervalSeconds { return .partial }
        return .wait
    }

    /// Whether a trailing buffer is worth transcribing when a recording stops.
    ///
    /// Anything shorter than one partial interval is almost certainly the tail
    /// of something already emitted.
    public func shouldFlush(bufferedSamples: Int) -> Bool {
        seconds(forSamples: bufferedSamples) >= min(1.0, partialIntervalSeconds)
    }
}
