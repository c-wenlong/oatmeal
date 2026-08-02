import AVFoundation

/// Converts whatever format a capture source hands us into the mono 16 kHz
/// Float32 that Whisper expects.
///
/// Both capture paths need this and they arrive in different formats — mic is
/// typically 44.1/48 kHz mono or stereo, ScreenCaptureKit gives 48 kHz stereo —
/// so it lives in one place rather than being duplicated per source.
final class AudioResampler {
    static let targetSampleRate: Double = 16_000

    private let converter: AVAudioConverter
    private let targetFormat: AVAudioFormat

    init?(from sourceFormat: AVAudioFormat) {
        guard
            let target = AVAudioFormat(
                commonFormat: .pcmFormatFloat32,
                sampleRate: Self.targetSampleRate,
                channels: 1,
                interleaved: false
            ),
            let converter = AVAudioConverter(from: sourceFormat, to: target)
        else { return nil }

        self.converter = converter
        self.targetFormat = target
    }

    /// Returns mono 16 kHz samples, or nil if conversion failed.
    func resample(_ buffer: AVAudioPCMBuffer) -> [Float]? {
        let ratio = targetFormat.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 1024

        guard
            let output = AVAudioPCMBuffer(
                pcmFormat: targetFormat, frameCapacity: capacity)
        else { return nil }

        var consumed = false
        var error: NSError?
        let status = converter.convert(to: output, error: &error) { _, outStatus in
            if consumed {
                outStatus.pointee = .noDataNow
                return nil
            }
            consumed = true
            outStatus.pointee = .haveData
            return buffer
        }

        guard status != .error, error == nil, output.frameLength > 0,
            let channel = output.floatChannelData?[0]
        else { return nil }

        return Array(UnsafeBufferPointer(start: channel, count: Int(output.frameLength)))
    }
}
