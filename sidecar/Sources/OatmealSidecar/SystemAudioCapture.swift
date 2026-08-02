import AVFoundation
import CoreGraphics
import ScreenCaptureKit

/// Captures everything the machine is playing, via ScreenCaptureKit in
/// audio-only mode.
///
/// We ask for a display filter because SCStream requires one, but shrink the
/// video to 2x2 at 1fps — we never read the video output. `capturesAudio` is the
/// only part we care about.
///
/// This needs Screen Recording permission even though no screen is recorded.
/// That is a macOS quirk with no way around it short of Core Audio process taps,
/// which can't capture the mic and are unreliable in production (SPEC section 4).
final class SystemAudioCapture: NSObject, SCStreamOutput {
    private var stream: SCStream?
    private var resampler: AudioResampler?
    private let onSamples: ([Float]) -> Void
    private let queue = DispatchQueue(label: "oatmeal.sidecar.system-audio")

    init(onSamples: @escaping ([Float]) -> Void) {
        self.onSamples = onSamples
    }

    func start() async throws {
        let content = try await SCShareableContent.excludingDesktopWindows(
            false, onScreenWindowsOnly: false)

        guard let display = content.displays.first else {
            throw CaptureError.noDisplay
        }

        let filter = SCContentFilter(
            display: display, excludingApplications: [], exceptingWindows: [])

        let config = SCStreamConfiguration()
        config.capturesAudio = true
        config.sampleRate = 48_000
        config.channelCount = 2
        // Don't transcribe our own output if this process ever makes noise.
        config.excludesCurrentProcessAudio = true
        // Video is mandatory but unused — make it as cheap as possible.
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(value: 1, timescale: 1)

        let stream = SCStream(filter: filter, configuration: config, delegate: nil)
        try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        try await stream.startCapture()
        self.stream = stream

        Log.info("System audio capture started (48 kHz stereo -> 16 kHz mono).")
    }

    func stop() async {
        guard let stream else { return }
        try? await stream.stopCapture()
        self.stream = nil
    }

    func stream(
        _ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard type == .audio, sampleBuffer.isValid, sampleBuffer.numSamples > 0 else {
            return
        }
        guard let pcm = sampleBuffer.asPCMBuffer else { return }

        if resampler == nil {
            resampler = AudioResampler(from: pcm.format)
            Log.info("System audio input format: \(pcm.format)")
        }
        guard let samples = resampler?.resample(pcm) else { return }
        onSamples(samples)
    }
}

extension CMSampleBuffer {
    /// Wraps the sample buffer's audio in an `AVAudioPCMBuffer` without copying.
    ///
    /// The underlying memory is only valid for the duration of the closure, so
    /// callers must consume (resample/copy) the result before returning.
    var asPCMBuffer: AVAudioPCMBuffer? {
        try? withAudioBufferList { audioBufferList, _ -> AVAudioPCMBuffer? in
            guard let absd = formatDescription?.audioStreamBasicDescription else {
                return nil
            }
            guard
                let format = AVAudioFormat(
                    standardFormatWithSampleRate: absd.mSampleRate,
                    channels: absd.mChannelsPerFrame)
            else { return nil }
            return AVAudioPCMBuffer(
                pcmFormat: format, bufferListNoCopy: audioBufferList.unsafePointer)
        }
    }
}

