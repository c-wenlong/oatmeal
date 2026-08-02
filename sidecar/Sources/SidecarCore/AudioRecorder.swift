import AVFoundation
import Foundation

public enum RecorderError: Error, CustomStringConvertible {
    case fileCreationFailed(String)

    public var description: String {
        switch self {
        case let .fileCreationFailed(reason):
            return "Could not create the recording file: \(reason)"
        }
    }
}

/// Writes the two capture streams to one two-channel file.
///
/// Channel layout is the contract with everything downstream:
///   channel 0 = mic    (you)
///   channel 1 = system (everyone else)
///
/// Keeping them as separate channels rather than mixing to mono is what lets a
/// future re-transcription recover attribution from the file alone — a mixdown
/// throws that away permanently.
public final class AudioRecorder {
    /// Compressed: raw 16 kHz float stereo is ~230 MB/hr, AAC is ~30 MB/hr, and
    /// audio is retained for days (SPEC section 11).
    /// Optional so it can be released deterministically. `AVAudioFile` only
    /// writes the container trailer when it deallocates, so a file that is still
    /// held is not yet readable — emitting its path before then hands consumers
    /// a truncated file.
    private var file: AVAudioFile?
    private let format: AVAudioFormat
    private var framesWritten: AVAudioFramePosition = 0

    public let url: URL

    public var durationMs: Int {
        Int((Double(framesWritten) / format.sampleRate) * 1000.0)
    }

    public init(url: URL, sampleRate: Double) throws {
        self.url = url

        guard
            let format = AVAudioFormat(
                commonFormat: .pcmFormatFloat32,
                sampleRate: sampleRate,
                channels: 2,
                interleaved: false)
        else {
            throw RecorderError.fileCreationFailed("unsupported stereo format")
        }
        self.format = format

        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: sampleRate,
            AVNumberOfChannelsKey: 2,
            AVEncoderAudioQualityKey: AVAudioQuality.medium.rawValue,
        ]

        do {
            // `commonFormat` describes what we hand in; the file itself is AAC.
            self.file = try AVAudioFile(
                forWriting: url,
                settings: settings,
                commonFormat: .pcmFormatFloat32,
                interleaved: false)
        } catch {
            throw RecorderError.fileCreationFailed(String(describing: error))
        }
    }

    /// Finalises the container. After this the file is complete on disk and
    /// further writes are ignored. Idempotent.
    public func close() {
        file = nil
    }

    /// Appends one aligned block. `mic` and `system` must be the same length —
    /// `StreamAligner` guarantees that.
    public func write(mic: [Float], system: [Float]) throws {
        precondition(mic.count == system.count, "channels must be aligned before writing")
        guard !mic.isEmpty else { return }

        guard
            let buffer = AVAudioPCMBuffer(
                pcmFormat: format, frameCapacity: AVAudioFrameCount(mic.count))
        else {
            throw RecorderError.fileCreationFailed("could not allocate a write buffer")
        }
        buffer.frameLength = AVAudioFrameCount(mic.count)

        guard let channels = buffer.floatChannelData else {
            throw RecorderError.fileCreationFailed("buffer has no float channels")
        }
        mic.withUnsafeBufferPointer { channels[0].update(from: $0.baseAddress!, count: mic.count) }
        system.withUnsafeBufferPointer {
            channels[1].update(from: $0.baseAddress!, count: system.count)
        }

        guard let file else { return }
        try file.write(from: buffer)
        framesWritten += AVAudioFramePosition(mic.count)
    }
}
