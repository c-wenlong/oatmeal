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

    func start() throws {
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)

        guard format.sampleRate > 0 else {
            throw CaptureError.micUnavailable
        }

        resampler = AudioResampler(from: format)
        Log.info("Mic input format: \(format)")

        input.installTap(onBus: 0, bufferSize: 4096, format: format) {
            [weak self] buffer, _ in
            guard let self, let samples = self.resampler?.resample(buffer) else {
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
}
