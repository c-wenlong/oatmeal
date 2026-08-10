import AVFoundation
import Foundation
import SidecarProtocol

/// Transcribing a file, for measurement.
///
/// The app only ever transcribes live audio, which is exactly what makes its
/// cost hard to reason about: at 1× realtime a model that is barely keeping up
/// and one with eight times the headroom look identical. Feeding a file as fast
/// as the machine will take it turns "does it work" into a number.
///
/// It goes through the same `Transcriber` the sidecar uses rather than calling
/// WhisperKit directly. A benchmark of a different code path measures a
/// different program.
enum Bench {
    /// Decodes any format Core Audio can read into the 16 kHz mono floats
    /// WhisperKit expects.
    ///
    /// Converted rather than assumed: an mp3 is 44.1 kHz stereo, and handing
    /// those samples over unconverted produces confident transcription of
    /// something nobody said.
    static func decode(_ url: URL, to sampleRate: Double = 16_000) throws -> [Float] {
        let file = try AVAudioFile(forReading: url)
        guard
            let target = AVAudioFormat(
                commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 1,
                interleaved: false),
            let converter = AVAudioConverter(from: file.processingFormat, to: target)
        else {
            throw NSError(
                domain: "bench", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "cannot convert \(file.processingFormat)"])
        }

        let ratio = sampleRate / file.processingFormat.sampleRate
        let capacity = AVAudioFrameCount(Double(file.length) * ratio) + 1024
        guard let out = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else {
            throw NSError(
                domain: "bench", code: 2,
                userInfo: [NSLocalizedDescriptionKey: "cannot allocate output buffer"])
        }

        var done = false
        var thrown: NSError?
        converter.convert(to: out, error: &thrown) { _, status in
            if done {
                status.pointee = .endOfStream
                return nil
            }
            guard
                let chunk = AVAudioPCMBuffer(
                    pcmFormat: file.processingFormat, frameCapacity: 8192)
            else {
                status.pointee = .endOfStream
                return nil
            }
            do {
                try file.read(into: chunk)
            } catch {
                status.pointee = .endOfStream
                return nil
            }
            if chunk.frameLength == 0 {
                done = true
                status.pointee = .endOfStream
                return nil
            }
            status.pointee = .haveData
            return chunk
        }
        if let thrown { throw thrown }

        guard let channel = out.floatChannelData?[0] else { return [] }
        return Array(UnsafeBufferPointer(start: channel, count: Int(out.frameLength)))
    }

    /// This process's physical footprint, which is what Activity Monitor shows.
    ///
    /// `phys_footprint` rather than resident size: CoreML maps large weight
    /// files, and resident size counts pages the kernel can evict at will —
    /// reporting it would overstate what the model actually costs.
    static func footprintBytes() -> UInt64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(
            MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? info.phys_footprint : 0
    }

    /// One line of machine-readable results, so a run can be diffed.
    struct Result: Codable {
        let model: String
        let audioSeconds: Double
        let loadSeconds: Double
        let transcribeSeconds: Double
        /// Audio seconds per wall second. Above 1 means faster than realtime.
        let realtimeFactor: Double
        let footprintMB: Double
        let windows: Int
        let characters: Int
    }

    /// Runs the file through the transcriber and reports what it cost.
    ///
    /// Windowed rather than one shot, at the size the live path uses: a single
    /// call over six minutes measures a batch job the app never runs.
    static func run(path: String, model: String, windowSeconds: Double) async -> Int32 {
        let url = URL(fileURLWithPath: path)
        let samples: [Float]
        do {
            samples = try decode(url)
        } catch {
            FileHandle.standardError.write(Data("bench: cannot read \(path): \(error)\n".utf8))
            return 2
        }
        let audioSeconds = Double(samples.count) / 16_000

        let transcriber = Transcriber(model: model)
        let loadStart = Date()
        do {
            try await transcriber.load(onEvent: { _ in })
        } catch {
            FileHandle.standardError.write(Data("bench: model failed to load: \(error)\n".utf8))
            return 3
        }
        let loadSeconds = Date().timeIntervalSince(loadStart)

        let windowSamples = Int(windowSeconds * 16_000)
        var text = ""
        var windows = 0
        let start = Date()
        var offset = 0
        while offset < samples.count {
            let end = min(offset + windowSamples, samples.count)
            if let piece = await transcriber.transcribe(Array(samples[offset..<end])) {
                text += piece + " "
            }
            windows += 1
            offset = end
        }
        let transcribeSeconds = Date().timeIntervalSince(start)

        let result = Result(
            model: model,
            audioSeconds: audioSeconds,
            loadSeconds: loadSeconds,
            transcribeSeconds: transcribeSeconds,
            realtimeFactor: transcribeSeconds > 0 ? audioSeconds / transcribeSeconds : 0,
            footprintMB: Double(footprintBytes()) / 1_048_576,
            windows: windows,
            characters: text.count)

        // The transcript to stdout, the numbers to stderr: the two get piped to
        // different places, and mixing them means neither can be used directly.
        FileHandle.standardOutput.write(Data((text + "\n").utf8))
        if let json = try? JSONEncoder().encode(result),
            let line = String(data: json, encoding: .utf8)
        {
            FileHandle.standardError.write(Data(("BENCH " + line + "\n").utf8))
        }
        return 0
    }
}
