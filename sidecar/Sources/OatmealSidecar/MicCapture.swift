import AVFoundation

/// Captures the microphone via AVAudioEngine.
///
/// Deliberately a completely separate pipeline from `SystemAudioCapture`. Two
/// independent streams is the whole point: `mic` is you, `system` is everyone
/// else, which gives us speaker attribution without a diarization model.
final class MicCapture {
    private let engine = AVAudioEngine()
    private var resampler: AudioResampler?
    private let onSamples: ([Float]) -> Void

    init(onSamples: @escaping ([Float]) -> Void) {
        self.onSamples = onSamples
    }

    /// Whether hardware echo cancellation is active on this run.
    ///
    /// Read after `start()`. False means the text-level `EchoSuppressor` is the
    /// only thing standing between speaker bleed and mis-attributed lines.
    private(set) var echoCancellationEnabled = false

    /// Opt-in hardware echo cancellation.
    ///
    /// **Off by default, and that is a measured decision rather than caution.**
    /// Apple's voice-processing IO unit is the textbook answer to speaker bleed —
    /// AEC referenced against what the machine is playing. On the development
    /// machine (macOS 26.5, built-in mic) it does not work, in three ways:
    ///
    /// - `setVoiceProcessingEnabled(true)` succeeds, but the input format changes
    ///   from `1 ch` to `7`–`9 ch`, and the count is not stable between runs.
    /// - Converting that multi-channel buffer to mono through `AVAudioConverter`
    ///   returns **silence**: feeding channel 0 a constant 1.0 and the rest zero
    ///   produces 0.0 out, so the inferred channel layout discards the mic.
    /// - With voice processing on, the input tap never fires at all — 0 frames
    ///   over 3 seconds, against 139,200 frames with it off. Adding an output
    ///   render path instead fails engine start with `-10875`.
    ///
    /// A silent microphone is far worse than bleed: bleed makes the transcript
    /// messy, silence loses the user's own half of the meeting entirely. So this
    /// stays behind a switch until it can be verified on hardware where it
    /// works, and `EchoSuppressor` carries the fix in the meantime.
    ///
    /// Set `OATMEAL_MIC_AEC=1` to try it.
    private static var aecRequested: Bool {
        ProcessInfo.processInfo.environment["OATMEAL_MIC_AEC"] == "1"
    }

    func start() throws {
        let input = engine.inputNode

        if Self.aecRequested {
            do {
                try input.setVoiceProcessingEnabled(true)
                echoCancellationEnabled = true
                Log.info("Mic echo cancellation enabled (OATMEAL_MIC_AEC=1).")
            } catch {
                Log.info("Mic echo cancellation unavailable (\(error)).")
            }
        }

        // Read *after* any voice-processing change: turning it on reconfigures
        // the node, so a format captured beforehand describes a graph that no
        // longer exists and the resampler would be built for the wrong input.
        let format = input.outputFormat(forBus: 0)

        guard format.sampleRate > 0 else {
            throw CaptureError.micUnavailable
        }

        // The multi-channel voice-processing layout does not survive
        // `AVAudioConverter`'s downmix — see the note above; it returns silence.
        // So when voice processing has widened the input, channel 0 is lifted
        // out by hand and the resampler is built for mono, never letting the
        // converter guess at a layout it gets wrong.
        let extractChannelZero = echoCancellationEnabled && format.channelCount > 1
        let resamplerFormat: AVAudioFormat
        if extractChannelZero,
            let mono = AVAudioFormat(
                commonFormat: .pcmFormatFloat32,
                sampleRate: format.sampleRate,
                channels: 1,
                interleaved: false)
        {
            resamplerFormat = mono
            Log.info(
                "Voice processing exposed \(format.channelCount) channels; "
                    + "taking channel 0 as the near-end mic.")
        } else {
            resamplerFormat = format
        }

        resampler = AudioResampler(from: resamplerFormat)
        Log.info("Mic input format: \(format)")

        input.installTap(onBus: 0, bufferSize: 4096, format: format) {
            [weak self] buffer, _ in
            guard let self else { return }
            let source = extractChannelZero ? Self.channelZero(of: buffer) : buffer
            guard let source, let samples = self.resampler?.resample(source) else {
                return
            }
            self.onSamples(samples)
        }

        engine.prepare()
        try engine.start()
        Log.info("Mic capture started.")
    }

    func stop() {
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
    }

    /// Copies channel 0 of a deinterleaved buffer into a mono buffer.
    ///
    /// Returns nil for anything that is not deinterleaved float — the caller
    /// then falls back to handing the original buffer to the converter, which is
    /// the correct path for an ordinary single-channel mic.
    private static func channelZero(of buffer: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
        guard let source = buffer.floatChannelData,
            let mono = AVAudioFormat(
                commonFormat: .pcmFormatFloat32,
                sampleRate: buffer.format.sampleRate,
                channels: 1,
                interleaved: false),
            let out = AVAudioPCMBuffer(
                pcmFormat: mono, frameCapacity: buffer.frameLength),
            let destination = out.floatChannelData
        else { return nil }

        out.frameLength = buffer.frameLength
        destination[0].update(from: source[0], count: Int(buffer.frameLength))
        return out
    }
}
