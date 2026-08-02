import Foundation

/// Decides whether a window of audio contains speech worth transcribing.
///
/// This exists because of a concrete finding from the G2 spike: Whisper does not
/// return nothing for silence — it confidently invents content. A near-silent
/// microphone produced "(upbeat music)", "(audience applauding)", "[BLANK_AUDIO]"
/// and "♪ ... ♪" over and over. Those would then be persisted as utterances and
/// fed to the summarizer as if someone had said them.
///
/// The spike's bare `rms > 0.001` gate was far too permissive. Two things fixed
/// it: a much higher energy threshold, and a *duration* requirement, so a single
/// keyboard click or a door closing doesn't qualify as speech.
public struct VoiceActivityDetector {
    /// Per-frame RMS a frame must exceed to count as active.
    public let energyThreshold: Float
    /// How much cumulative active audio a window needs before it is transcribed.
    public let minimumSpeechSeconds: Double
    public let frameSeconds: Double
    public let sampleRate: Double

    public init(
        energyThreshold: Float = 0.012,
        minimumSpeechSeconds: Double = 0.25,
        frameSeconds: Double = 0.02,
        sampleRate: Double = 16_000
    ) {
        self.energyThreshold = energyThreshold
        self.minimumSpeechSeconds = minimumSpeechSeconds
        self.frameSeconds = frameSeconds
        self.sampleRate = sampleRate
    }

    private var frameLength: Int { max(1, Int(frameSeconds * sampleRate)) }

    /// Seconds of the window that look like speech.
    public func speechSeconds(in samples: [Float]) -> Double {
        guard !samples.isEmpty else { return 0 }
        let length = frameLength
        var activeFrames = 0
        var index = 0

        while index + length <= samples.count {
            var sum: Float = 0
            for offset in index..<(index + length) {
                let value = samples[offset]
                sum += value * value
            }
            if (sum / Float(length)).squareRoot() > energyThreshold {
                activeFrames += 1
            }
            index += length
        }

        return Double(activeFrames) * frameSeconds
    }

    /// Whether this window should be sent to the ASR model at all.
    public func containsSpeech(_ samples: [Float]) -> Bool {
        speechSeconds(in: samples) >= minimumSpeechSeconds
    }
}
