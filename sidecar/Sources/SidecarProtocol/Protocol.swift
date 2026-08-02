import Foundation

/// Wire protocol between the Rust core and this sidecar.
///
/// Newline-delimited JSON over stdio, one object per line. Both sides must agree
/// on `PROTOCOL_VERSION`; the Rust supervisor refuses a sidecar that announces a
/// version it doesn't know, which turns "silently wrong field names after an
/// upgrade" into a loud startup failure.
///
/// Mirrored by `src-tauri/src/sidecar/protocol.rs`.
public let PROTOCOL_VERSION = 1

// MARK: - Commands (Rust -> sidecar)

public enum AudioSource: String, Codable, Sendable, CaseIterable {
    case mic
    case system
}

/// TCC authorisation state for one capability.
///
/// `undetermined` matters as its own case: it's the only state where prompting
/// the user can still succeed. Once macOS records a denial, the prompt never
/// reappears and the only route is System Settings — so the UI has to say
/// something different.
public enum PermissionState: String, Codable, Sendable {
    case granted
    case denied
    case undetermined
}

public enum SidecarCommand: Sendable, Equatable {
    case start(meetingId: String, sources: [AudioSource])
    case stop
    case ping
    /// Begins capture into the rolling pre-roll buffer without writing anything
    /// to disk. `start` then opens with whatever was already being said.
    case arm
    /// Tears capture down entirely, so nothing is being listened to.
    case disarm
    /// Reports permission state. With `request: true` it also prompts, which is
    /// only useful while a capability is still `undetermined`.
    case permissions(request: Bool)
}

extension SidecarCommand: Codable {
    private enum CodingKeys: String, CodingKey {
        case cmd
        case meetingId = "meeting_id"
        case sources
        case request
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let cmd = try container.decode(String.self, forKey: .cmd)
        switch cmd {
        case "start":
            let meetingId = try container.decode(String.self, forKey: .meetingId)
            let sources =
                try container.decodeIfPresent([AudioSource].self, forKey: .sources)
                ?? AudioSource.allCases
            self = .start(meetingId: meetingId, sources: sources)
        case "stop":
            self = .stop
        case "ping":
            self = .ping
        case "arm":
            self = .arm
        case "disarm":
            self = .disarm
        case "permissions":
            self = .permissions(
                request: try container.decodeIfPresent(Bool.self, forKey: .request) ?? false)
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .cmd, in: container,
                debugDescription: "unknown command '\(cmd)'")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .start(meetingId, sources):
            try container.encode("start", forKey: .cmd)
            try container.encode(meetingId, forKey: .meetingId)
            try container.encode(sources, forKey: .sources)
        case .stop:
            try container.encode("stop", forKey: .cmd)
        case .ping:
            try container.encode("ping", forKey: .cmd)
        case .arm:
            try container.encode("arm", forKey: .cmd)
        case .disarm:
            try container.encode("disarm", forKey: .cmd)
        case let .permissions(request):
            try container.encode("permissions", forKey: .cmd)
            try container.encode(request, forKey: .request)
        }
    }
}

// MARK: - Events (sidecar -> Rust)

public enum SidecarEvent: Sendable, Equatable {
    /// First line the sidecar ever writes. The supervisor waits for it before
    /// considering a spawn successful.
    case ready(version: String, protocolVersion: Int)
    /// In-flight text for the live UI. Superseded by a later `final`; never persisted.
    case partial(source: AudioSource, text: String, t0: Int, t1: Int)
    /// Settled text. This is what becomes a row in `utterances`.
    case final(source: AudioSource, text: String, t0: Int, t1: Int, conf: Double?)
    /// Input levels for the recording indicator.
    case level(mic: Double, system: Double)
    case stopped(audioPath: String?, durationMs: Int)
    case error(message: String, fatal: Bool)
    case pong
    /// Current TCC state. `needsRelaunch` is set when Screen Recording reads as
    /// granted but the running process is still holding a stale denial — macOS
    /// only hands the new grant to a fresh process.
    case permissions(
        microphone: PermissionState, screenRecording: PermissionState, needsRelaunch: Bool)
    /// ASR model lifecycle. `progress` is 0...1 while downloading.
    case model(name: String, state: ModelState, progress: Double?, message: String?)
}

public enum ModelState: String, Codable, Sendable {
    case downloading
    case loading
    case ready
    case failed
}

extension SidecarEvent: Codable {
    private enum CodingKeys: String, CodingKey {
        case ev, version, text, source, t0, t1, conf, mic, system, message, fatal
        case name, state, progress
        case protocolVersion = "protocol"
        case audioPath = "audio_path"
        case durationMs = "duration_ms"
        case microphone
        case screenRecording = "screen_recording"
        case needsRelaunch = "needs_relaunch"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let ev = try c.decode(String.self, forKey: .ev)
        switch ev {
        case "ready":
            self = .ready(
                version: try c.decode(String.self, forKey: .version),
                protocolVersion: try c.decode(Int.self, forKey: .protocolVersion))
        case "partial":
            self = .partial(
                source: try c.decode(AudioSource.self, forKey: .source),
                text: try c.decode(String.self, forKey: .text),
                t0: try c.decode(Int.self, forKey: .t0),
                t1: try c.decode(Int.self, forKey: .t1))
        case "final":
            self = .final(
                source: try c.decode(AudioSource.self, forKey: .source),
                text: try c.decode(String.self, forKey: .text),
                t0: try c.decode(Int.self, forKey: .t0),
                t1: try c.decode(Int.self, forKey: .t1),
                conf: try c.decodeIfPresent(Double.self, forKey: .conf))
        case "level":
            self = .level(
                mic: try c.decode(Double.self, forKey: .mic),
                system: try c.decode(Double.self, forKey: .system))
        case "stopped":
            self = .stopped(
                audioPath: try c.decodeIfPresent(String.self, forKey: .audioPath),
                durationMs: try c.decode(Int.self, forKey: .durationMs))
        case "error":
            self = .error(
                message: try c.decode(String.self, forKey: .message),
                fatal: try c.decodeIfPresent(Bool.self, forKey: .fatal) ?? false)
        case "pong":
            self = .pong
        case "permissions":
            self = .permissions(
                microphone: try c.decode(PermissionState.self, forKey: .microphone),
                screenRecording: try c.decode(
                    PermissionState.self, forKey: .screenRecording),
                needsRelaunch: try c.decodeIfPresent(Bool.self, forKey: .needsRelaunch)
                    ?? false)
        case "model":
            self = .model(
                name: try c.decode(String.self, forKey: .name),
                state: try c.decode(ModelState.self, forKey: .state),
                progress: try c.decodeIfPresent(Double.self, forKey: .progress),
                message: try c.decodeIfPresent(String.self, forKey: .message))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .ev, in: c, debugDescription: "unknown event '\(ev)'")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .ready(version, protocolVersion):
            try c.encode("ready", forKey: .ev)
            try c.encode(version, forKey: .version)
            try c.encode(protocolVersion, forKey: .protocolVersion)
        case let .partial(source, text, t0, t1):
            try c.encode("partial", forKey: .ev)
            try c.encode(source, forKey: .source)
            try c.encode(text, forKey: .text)
            try c.encode(t0, forKey: .t0)
            try c.encode(t1, forKey: .t1)
        case let .final(source, text, t0, t1, conf):
            try c.encode("final", forKey: .ev)
            try c.encode(source, forKey: .source)
            try c.encode(text, forKey: .text)
            try c.encode(t0, forKey: .t0)
            try c.encode(t1, forKey: .t1)
            try c.encodeIfPresent(conf, forKey: .conf)
        case let .level(mic, system):
            try c.encode("level", forKey: .ev)
            try c.encode(mic, forKey: .mic)
            try c.encode(system, forKey: .system)
        case let .stopped(audioPath, durationMs):
            try c.encode("stopped", forKey: .ev)
            try c.encodeIfPresent(audioPath, forKey: .audioPath)
            try c.encode(durationMs, forKey: .durationMs)
        case let .error(message, fatal):
            try c.encode("error", forKey: .ev)
            try c.encode(message, forKey: .message)
            try c.encode(fatal, forKey: .fatal)
        case .pong:
            try c.encode("pong", forKey: .ev)
        case let .permissions(microphone, screenRecording, needsRelaunch):
            try c.encode("permissions", forKey: .ev)
            try c.encode(microphone, forKey: .microphone)
            try c.encode(screenRecording, forKey: .screenRecording)
            try c.encode(needsRelaunch, forKey: .needsRelaunch)
        case let .model(name, state, progress, message):
            try c.encode("model", forKey: .ev)
            try c.encode(name, forKey: .name)
            try c.encode(state, forKey: .state)
            try c.encodeIfPresent(progress, forKey: .progress)
            try c.encodeIfPresent(message, forKey: .message)
        }
    }
}

// MARK: - Line codec

public enum WireCodec {
    /// JSON must stay on one line; a pretty-printed encoder would silently break
    /// the framing.
    public static func encode(_ event: SidecarEvent) throws -> String {
        let data = try JSONEncoder().encode(event)
        return String(decoding: data, as: UTF8.self)
    }

    public static func decodeCommand(_ line: String) throws -> SidecarCommand {
        let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
        return try JSONDecoder().decode(
            SidecarCommand.self, from: Data(trimmed.utf8))
    }

    public static func decodeEvent(_ line: String) throws -> SidecarEvent {
        let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
        return try JSONDecoder().decode(SidecarEvent.self, from: Data(trimmed.utf8))
    }
}
