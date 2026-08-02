import Foundation

/// A deterministic, hard-coded meeting used until real capture lands in G6/G7.
///
/// Deterministic on purpose: the Rust integration test asserts the exact
/// sequence, so any accidental change to the wire format fails a test rather
/// than quietly producing different-looking events.
///
/// The shape mirrors what real ASR will emit — `partial`s that get superseded by
/// a `final`, interleaved across both sources, with `level` updates in between.
public enum ScriptedSession {
    public struct Step: Sendable {
        public let delayMs: Int
        public let event: SidecarEvent
    }

    public static let transcript: [Step] = [
        Step(delayMs: 60, event: .level(mic: 0.02, system: 0.31)),
        Step(
            delayMs: 120,
            event: .partial(source: .system, text: "so the deadline", t0: 400, t1: 1_500)),
        Step(
            delayMs: 180,
            event: .final(
                source: .system,
                text: "So the deadline for the migration is the fourteenth.",
                t0: 400, t1: 3_200, conf: 0.93)),
        Step(delayMs: 60, event: .level(mic: 0.44, system: 0.03)),
        Step(
            delayMs: 120,
            event: .partial(source: .mic, text: "got it I'll own", t0: 3_400, t1: 4_400)),
        Step(
            delayMs: 180,
            event: .final(
                source: .mic,
                text: "Got it, I'll own the rollback plan.",
                t0: 3_400, t1: 5_600, conf: 0.88)),
        Step(delayMs: 60, event: .level(mic: 0.01, system: 0.28)),
        Step(
            delayMs: 180,
            event: .final(
                source: .system,
                text: "Perfect, let's review it on Thursday.",
                t0: 5_800, t1: 7_900, conf: 0.91)),
    ]

    /// Only the `final` events, which are the ones that become `utterances` rows.
    public static var finals: [SidecarEvent] {
        transcript.map(\.event).filter {
            if case .final = $0 { return true }
            return false
        }
    }

    public static let durationMs = 7_900
}
