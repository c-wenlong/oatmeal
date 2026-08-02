import Foundation
import SidecarCore
import SidecarProtocol
import WhisperKit

/// Wraps WhisperKit: loads the model, verifies it, and transcribes windows.
///
/// One shared instance serves both streams. Two model instances would double
/// memory (~500 MB each for `small.en`) for no accuracy gain — the G2 spike
/// confirmed a single instance keeps up with both at these window sizes.
actor Transcriber {
    private var whisper: WhisperKit?
    private let modelName: String

    init(model: String) {
        self.modelName = model
    }

    var isLoaded: Bool { whisper != nil }
    var model: String { modelName }

    /// CoreML bundles that must be present in `modelFolder`.
    ///
    /// Note the tokenizer is deliberately *not* checked here: WhisperKit keeps
    /// it under a separate `openai/whisper-*` path, not in `modelFolder`, so
    /// looking for `tokenizer.json` in this directory rejects healthy models.
    /// The warm-up below covers the tokenizer properly.
    private static let requiredBundles = [
        "AudioEncoder.mlmodelc",
        "MelSpectrogram.mlmodelc",
        "TextDecoder.mlmodelc",
    ]

    /// Loads the model, reporting progress. Verifies the download before use.
    func load(onEvent: @escaping @Sendable (SidecarEvent) -> Void) async throws {
        if whisper != nil { return }

        onEvent(.model(name: modelName, state: .downloading, progress: nil, message: nil))

        let config = WhisperKitConfig(model: modelName, download: true)
        let kit: WhisperKit
        do {
            kit = try await WhisperKit(config)
        } catch {
            onEvent(
                .model(
                    name: modelName, state: .failed, progress: nil,
                    message: "\(error)"))
            throw error
        }

        onEvent(.model(name: modelName, state: .loading, progress: nil, message: nil))

        if let folder = kit.modelFolder, let problem = Self.verify(folder: folder) {
            onEvent(
                .model(name: modelName, state: .failed, progress: nil, message: problem))
            throw CaptureError.modelUnusable(problem)
        }

        // Warm-up: transcribe a moment of silence. This is the real integrity
        // check — it exercises the tokenizer and the whole CoreML pipeline, so a
        // half-downloaded model fails *here* rather than mid-meeting, which is
        // what happened in the G2 spike (`configurationMissing("tokenizer.json")`).
        // It also pays the first-run compilation cost before anyone is talking.
        do {
            _ = try await kit.transcribe(
                audioArray: [Float](repeating: 0, count: 16_000))
        } catch {
            let problem = "model failed its warm-up: \(error)"
            onEvent(
                .model(name: modelName, state: .failed, progress: nil, message: problem))
            throw CaptureError.modelUnusable(problem)
        }

        whisper = kit
        onEvent(.model(name: modelName, state: .ready, progress: nil, message: nil))
        Log.info("model \(modelName) ready")
    }

    /// Returns a description of what's missing, or nil when the model is intact.
    ///
    /// Checked at load rather than at first use, so an incomplete download is a
    /// startup failure the user can act on instead of a broken recording.
    nonisolated static func verify(folder: URL) -> String? {
        let manager = FileManager.default
        for bundle in requiredBundles {
            let path = folder.appendingPathComponent(bundle).path
            var isDirectory: ObjCBool = false
            if !manager.fileExists(atPath: path, isDirectory: &isDirectory)
                || !isDirectory.boolValue
            {
                return "\(bundle) is missing — the model download is incomplete"
            }
        }
        // A stray `.incomplete` temp file means the download was interrupted.
        if let contents = try? manager.contentsOfDirectory(atPath: folder.path),
            contents.contains(where: { $0.hasSuffix(".incomplete") })
        {
            return "the model download was interrupted"
        }
        return nil
    }

    /// Transcribes one window. Returns nil when there is nothing usable.
    func transcribe(_ samples: [Float]) async -> String? {
        guard let whisper else { return nil }
        guard !samples.isEmpty else { return nil }

        do {
            let results = try await whisper.transcribe(audioArray: samples)
            let text = results.map(\.text).joined(separator: " ")
            // Every model output passes the G2 filter before it can become an
            // utterance — see spike/FINDINGS.md.
            return TranscriptFilter.clean(text)
        } catch {
            Log.error("transcription failed: \(error)")
            return nil
        }
    }
}
