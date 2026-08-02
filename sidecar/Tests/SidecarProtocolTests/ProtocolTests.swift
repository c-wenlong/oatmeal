import XCTest

@testable import SidecarProtocol

/// The wire format is a contract with `src-tauri/src/sidecar/protocol.rs`. These
/// tests pin the exact JSON keys, because a rename on this side would otherwise
/// only show up as events silently vanishing at runtime.
final class ProtocolTests: XCTestCase {

    // MARK: Commands

    func testDecodesStartWithExplicitSources() throws {
        let cmd = try WireCodec.decodeCommand(
            #"{"cmd":"start","meeting_id":"m1","sources":["mic"]}"#)
        XCTAssertEqual(cmd, .start(meetingId: "m1", sources: [.mic]))
    }

    func testStartDefaultsToBothSourcesWhenOmitted() throws {
        // Both streams is the useful default; omitting `sources` must not mean
        // "capture nothing".
        let cmd = try WireCodec.decodeCommand(#"{"cmd":"start","meeting_id":"m1"}"#)
        XCTAssertEqual(cmd, .start(meetingId: "m1", sources: [.mic, .system]))
    }

    func testDecodesStopAndPing() throws {
        XCTAssertEqual(try WireCodec.decodeCommand(#"{"cmd":"stop"}"#), .stop)
        XCTAssertEqual(try WireCodec.decodeCommand(#"{"cmd":"ping"}"#), .ping)
    }

    func testRejectsUnknownCommand() {
        XCTAssertThrowsError(try WireCodec.decodeCommand(#"{"cmd":"selfdestruct"}"#))
    }

    func testRejectsMalformedJson() {
        XCTAssertThrowsError(try WireCodec.decodeCommand("{not json"))
    }

    func testRejectsStartWithoutMeetingId() {
        XCTAssertThrowsError(try WireCodec.decodeCommand(#"{"cmd":"start"}"#))
    }

    func testToleratesSurroundingWhitespace() throws {
        XCTAssertEqual(try WireCodec.decodeCommand("  {\"cmd\":\"stop\"}  \n"), .stop)
    }

    // MARK: Events

    func testEncodedEventsAreSingleLine() throws {
        // Framing is newline-delimited; an embedded newline would desync the
        // parser for the rest of the session.
        for step in ScriptedSession.transcript {
            let line = try WireCodec.encode(step.event)
            XCTAssertFalse(line.contains("\n"), "event encoded across lines: \(line)")
        }
    }

    func testFinalEventUsesTheAgreedKeys() throws {
        let line = try WireCodec.encode(
            .final(source: .mic, text: "hello", t0: 1, t1: 2, conf: 0.5))
        let json =
            try JSONSerialization.jsonObject(with: Data(line.utf8)) as! [String: Any]
        XCTAssertEqual(json["ev"] as? String, "final")
        XCTAssertEqual(json["source"] as? String, "mic")
        XCTAssertEqual(json["text"] as? String, "hello")
        XCTAssertEqual(json["t0"] as? Int, 1)
        XCTAssertEqual(json["t1"] as? Int, 2)
        XCTAssertEqual(json["conf"] as? Double, 0.5)
    }

    func testReadyUsesProtocolKeyNotProtocolVersion() throws {
        let line = try WireCodec.encode(.ready(version: "0.1.0", protocolVersion: 1))
        let json =
            try JSONSerialization.jsonObject(with: Data(line.utf8)) as! [String: Any]
        XCTAssertEqual(json["protocol"] as? Int, 1)
        XCTAssertNil(json["protocolVersion"])
    }

    func testStoppedUsesSnakeCaseKeys() throws {
        let line = try WireCodec.encode(.stopped(audioPath: "/tmp/a.m4a", durationMs: 5))
        let json =
            try JSONSerialization.jsonObject(with: Data(line.utf8)) as! [String: Any]
        XCTAssertEqual(json["audio_path"] as? String, "/tmp/a.m4a")
        XCTAssertEqual(json["duration_ms"] as? Int, 5)
    }

    func testPermissionsCommandDefaultsToQueryNotPrompt() throws {
        // A bare `{"cmd":"permissions"}` must not pop system dialogs at users.
        let cmd = try WireCodec.decodeCommand(#"{"cmd":"permissions"}"#)
        XCTAssertEqual(cmd, .permissions(request: false))
    }

    func testPermissionsEventUsesSnakeCaseKeys() throws {
        let line = try WireCodec.encode(
            .permissions(microphone: .granted, screenRecording: .denied, needsRelaunch: true))
        let json =
            try JSONSerialization.jsonObject(with: Data(line.utf8)) as! [String: Any]
        XCTAssertEqual(json["microphone"] as? String, "granted")
        XCTAssertEqual(json["screen_recording"] as? String, "denied")
        XCTAssertEqual(json["needs_relaunch"] as? Bool, true)
    }

    func testEveryEventRoundTrips() throws {
        let events: [SidecarEvent] = [
            .ready(version: "0.1.0", protocolVersion: PROTOCOL_VERSION),
            .partial(source: .system, text: "partial text", t0: 0, t1: 10),
            .final(source: .mic, text: "final text", t0: 0, t1: 20, conf: 0.9),
            .final(source: .mic, text: "no confidence", t0: 0, t1: 20, conf: nil),
            .level(mic: 0.1, system: 0.2),
            .stopped(audioPath: nil, durationMs: 100),
            .error(message: "boom", fatal: true),
            .pong,
            .permissions(
                microphone: .granted, screenRecording: .undetermined, needsRelaunch: false),
            .model(name: "small.en", state: .downloading, progress: 0.5, message: nil),
            .model(name: "small.en", state: .ready, progress: nil, message: nil),
        ]

        for event in events {
            let decoded = try WireCodec.decodeEvent(try WireCodec.encode(event))
            XCTAssertEqual(decoded, event)
        }
    }

    func testRejectsUnknownEvent() {
        XCTAssertThrowsError(try WireCodec.decodeEvent(#"{"ev":"teleport"}"#))
    }

    // MARK: Scripted session

    func testScriptedSessionHasFinalsOnBothSources() {
        let sources: [AudioSource] = ScriptedSession.finals.compactMap {
            if case let .final(source, _, _, _, _) = $0 { return source }
            return nil
        }
        // Attribution is the point of two streams — a fixture that only ever
        // emits one source would let a broken `source` field pass unnoticed.
        XCTAssertTrue(sources.contains(.mic))
        XCTAssertTrue(sources.contains(.system))
    }

    func testScriptedTimestampsAreMonotonic() {
        var last = -1
        for step in ScriptedSession.transcript {
            switch step.event {
            case let .partial(_, _, t0, t1), let .final(_, _, t0, t1, _):
                XCTAssertLessThanOrEqual(t0, t1)
                XCTAssertGreaterThanOrEqual(t0, last)
                last = t0
            default:
                continue
            }
        }
    }
}
