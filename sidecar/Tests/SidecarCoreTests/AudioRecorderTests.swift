import AVFoundation
import XCTest

@testable import SidecarCore

/// The two-channel layout is a contract with everything downstream: a
/// re-transcription months from now recovers speaker attribution from the file
/// alone. These tests pin it without needing a microphone or a display.
final class AudioRecorderTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("oatmeal-recorder-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    private func url(_ name: String) -> URL {
        directory.appendingPathComponent(name)
    }

    /// A tone, so a channel's content is identifiable after encoding.
    private func tone(hz: Double, seconds: Double, rate: Double, amplitude: Float = 0.5)
        -> [Float]
    {
        let count = Int(seconds * rate)
        return (0..<count).map { i in
            amplitude * Float(sin(2.0 * Double.pi * hz * Double(i) / rate))
        }
    }

    func testWritesATwoChannelFile() throws {
        let output = url("stereo.m4a")
        let recorder = try AudioRecorder(url: output, sampleRate: 16_000)
        try recorder.write(
            mic: tone(hz: 440, seconds: 0.5, rate: 16_000),
            system: tone(hz: 880, seconds: 0.5, rate: 16_000))
        recorder.close()

        XCTAssertTrue(FileManager.default.fileExists(atPath: output.path))

        let file = try AVAudioFile(forReading: output)
        XCTAssertEqual(
            file.fileFormat.channelCount, 2,
            "a mixdown to mono would destroy speaker attribution permanently")
        XCTAssertEqual(file.fileFormat.sampleRate, 16_000)
    }

    func testChannelsCarryDifferentSignals() throws {
        let output = url("distinct.m4a")
        let recorder = try AudioRecorder(url: output, sampleRate: 16_000)
        // Mic loud, system silent: the decoded file must preserve that asymmetry
        // rather than mixing them together.
        try recorder.write(
            mic: tone(hz: 440, seconds: 1.0, rate: 16_000, amplitude: 0.8),
            system: [Float](repeating: 0, count: 16_000))
        recorder.close()

        let file = try AVAudioFile(forReading: output)
        let format = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: file.fileFormat.sampleRate,
            channels: 2,
            interleaved: false)!
        let buffer = AVAudioPCMBuffer(
            pcmFormat: format, frameCapacity: AVAudioFrameCount(file.length))!
        try file.read(into: buffer)

        let channels = buffer.floatChannelData!
        let frames = Int(buffer.frameLength)
        func rms(_ pointer: UnsafeMutablePointer<Float>) -> Float {
            var sum: Float = 0
            for i in 0..<frames { sum += pointer[i] * pointer[i] }
            return (sum / Float(frames)).squareRoot()
        }

        let micEnergy = rms(channels[0])
        let systemEnergy = rms(channels[1])
        XCTAssertGreaterThan(micEnergy, 0.1, "mic channel lost its signal")
        XCTAssertLessThan(
            systemEnergy, micEnergy / 4,
            "channels appear mixed — the silent channel picked up the loud one")
    }

    func testDurationTracksWhatWasWritten() throws {
        let recorder = try AudioRecorder(url: url("duration.m4a"), sampleRate: 16_000)
        XCTAssertEqual(recorder.durationMs, 0)

        let silence = [Float](repeating: 0, count: 16_000)
        try recorder.write(mic: silence, system: silence)
        XCTAssertEqual(recorder.durationMs, 1_000)

        try recorder.write(mic: silence, system: silence)
        XCTAssertEqual(recorder.durationMs, 2_000)
    }

    func testWritesAfterCloseAreIgnoredRatherThanCrashing() throws {
        let recorder = try AudioRecorder(url: url("closed.m4a"), sampleRate: 16_000)
        let silence = [Float](repeating: 0, count: 160)
        try recorder.write(mic: silence, system: silence)
        recorder.close()
        // A late flush racing `stop` must not take the process down.
        try recorder.write(mic: silence, system: silence)
        recorder.close()
    }

    func testWritingNothingIsHarmless() throws {
        let recorder = try AudioRecorder(url: url("empty.m4a"), sampleRate: 16_000)
        try recorder.write(mic: [], system: [])
        XCTAssertEqual(recorder.durationMs, 0)
    }

    func testMisalignedChannelsAreARejectedProgrammerError() throws {
        // `StreamAligner` guarantees equal lengths; if that ever regresses we
        // want a hard failure rather than a silently skewed file.
        let recorder = try AudioRecorder(url: url("skew.m4a"), sampleRate: 16_000)
        // Not testable via XCTAssertThrows (it's a precondition), so assert the
        // guarantee at its source instead.
        var aligner = StreamAligner()
        aligner.push(mic: [1, 2, 3])
        aligner.push(system: [1])
        let block = aligner.pull(frames: 3)
        XCTAssertEqual(block?.mic.count, block?.system.count)
        try recorder.write(mic: block!.mic, system: block!.system)
    }

    func testCompressionKeepsFilesReasonablySized() throws {
        let output = url("size.m4a")
        let recorder = try AudioRecorder(url: output, sampleRate: 16_000)
        // 10 seconds of tone.
        for _ in 0..<10 {
            try recorder.write(
                mic: tone(hz: 440, seconds: 1.0, rate: 16_000),
                system: tone(hz: 880, seconds: 1.0, rate: 16_000))
        }
        recorder.close()

        let bytes = try FileManager.default
            .attributesOfItem(atPath: output.path)[.size] as! Int
        // Raw float32 stereo would be 1.28 MB for 10s; AAC must be far smaller,
        // since audio is retained for days (SPEC section 11).
        XCTAssertLessThan(bytes, 200_000, "audio does not appear to be compressed")
        XCTAssertGreaterThan(bytes, 1_000, "file is suspiciously empty")
    }
}
