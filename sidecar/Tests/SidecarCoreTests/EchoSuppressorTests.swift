import XCTest

@testable import SidecarCore

final class EchoSuppressorTests: XCTestCase {

    /// The G2 finding, reproduced: the same sentence on both channels, the mic
    /// copy mangled by the room.
    func testSuppressesAGarbledCopyOfWhatTheSpeakersPlayed() {
        var suppressor = EchoSuppressor()
        suppressor.noteSystem(
            text: "So the deadline for the migration is the fourteenth of March.", endedAt: 12_000)

        XCTAssertTrue(
            suppressor.isEcho(
                micText: "so the deadline for the migration is the fourteenth", t0: 12_400),
            "a clipped, unpunctuated copy of the system line should be recognised")
    }

    func testKeepsGenuineSpeechThatSharesNoWording() {
        var suppressor = EchoSuppressor()
        suppressor.noteSystem(
            text: "So the deadline for the migration is the fourteenth of March.", endedAt: 12_000)

        XCTAssertFalse(
            suppressor.isEcho(micText: "I will own the rollback plan myself", t0: 12_500))
    }

    /// The failure mode that matters most: deleting something the user said.
    func testKeepsShortAgreementEvenWhenItEchoesTheOtherChannel() {
        var suppressor = EchoSuppressor()
        suppressor.noteSystem(text: "Are we agreed on Thursday?", endedAt: 5_000)

        // A person genuinely saying this is indistinguishable from bleed on the
        // words alone, so it must be kept.
        XCTAssertFalse(suppressor.isEcho(micText: "agreed on Thursday", t0: 5_400))
    }

    func testKeepsALineThatArrivesLongAfterTheSystemSaidIt() {
        var suppressor = EchoSuppressor()
        suppressor.noteSystem(
            text: "The security review is the blocker right now", endedAt: 10_000)

        // Well outside the window: someone circling back to the same topic later
        // is a real utterance, not an echo.
        XCTAssertFalse(
            suppressor.isEcho(
                micText: "the security review is the blocker right now", t0: 90_000))
    }

    func testASystemLineStopsBeingACandidateOnceItAgesOut() {
        var suppressor = EchoSuppressor(windowMs: 3_000)
        suppressor.noteSystem(text: "we are putting a hiring freeze in place", endedAt: 1_000)

        XCTAssertFalse(
            suppressor.isEcho(micText: "we are putting a hiring freeze in place", t0: 9_000))
    }

    func testPartialOverlapBelowThresholdIsKept() {
        // Two sentences about the same subject share some words. That is not an
        // echo, and treating it as one would delete half a conversation.
        var suppressor = EchoSuppressor()
        suppressor.noteSystem(
            text: "the migration deadline is the fourteenth and it has not moved", endedAt: 8_000)

        XCTAssertFalse(
            suppressor.isEcho(
                micText: "who is handling the migration testing before we ship it",
                t0: 8_500))
    }

    func testEmptyAndPunctuationOnlyLinesAreHarmless() {
        var suppressor = EchoSuppressor()
        suppressor.noteSystem(text: "...", endedAt: 1_000)
        XCTAssertFalse(suppressor.isEcho(micText: "", t0: 1_100))
        XCTAssertFalse(suppressor.isEcho(micText: "!!!", t0: 1_100))
    }

    func testContainmentIgnoresCaseAndPunctuation() {
        let a = EchoSuppressor.tokens("The deadline, is Thursday!")
        let b = EchoSuppressor.tokens("the deadline is thursday")
        XCTAssertEqual(EchoSuppressor.containment(a, b), 1.0, accuracy: 0.001)
    }

    func testSuppressionSurvivesRepeatedSystemLines() {
        // The system channel keeps producing finals; the suppressor must not
        // grow without bound or lose older-but-still-recent candidates.
        var suppressor = EchoSuppressor()
        for index in 0..<50 {
            suppressor.noteSystem(text: "line number \(index) about the roadmap", endedAt: index * 100)
        }
        XCTAssertTrue(
            suppressor.isEcho(micText: "line number 49 about the roadmap", t0: 4_950))
    }

    // MARK: - Routing

    /// The inversion that would be invisible until someone read a transcript.
    func testSystemLinesAreNeverSuppressed() {
        var suppressor = EchoSuppressor()
        XCTAssertTrue(
            suppressor.admit(
                isMic: false, text: "the deadline for the migration is the fourteenth",
                t0: 1_000, t1: 1_000))
        // Even an identical system line arriving twice stays — the far end
        // repeating itself is not bleed.
        XCTAssertTrue(
            suppressor.admit(
                isMic: false, text: "the deadline for the migration is the fourteenth",
                t0: 2_000, t1: 2_000))
    }

    func testAdmitDropsTheMicCopyOfAnAdmittedSystemLine() {
        var suppressor = EchoSuppressor()
        XCTAssertTrue(
            suppressor.admit(
                isMic: false, text: "we are putting a hiring freeze in place until Q3",
                t0: 1_000, t1: 1_000))
        XCTAssertFalse(
            suppressor.admit(
                isMic: true, text: "we are putting a hiring freeze in place until q3",
                t0: 1_600, t1: 1_600),
            "the mic copy of what the speakers just played should not reach the wire")
    }

    func testAMicLineWithNoSystemLineBeforeItIsAlwaysKept() {
        // In-person meetings have no system audio at all; nothing may be dropped.
        var suppressor = EchoSuppressor()
        XCTAssertTrue(
            suppressor.admit(
                isMic: true, text: "I will own the rollback plan myself", t0: 5_000, t1: 5_000))
    }

    func testMicLinesDoNotBecomeEchoCandidatesForOtherMicLines() {
        // Only the system channel can bleed into the mic. If mic lines seeded
        // the candidate set, a speaker repeating themselves would silence them.
        var suppressor = EchoSuppressor()
        XCTAssertTrue(
            suppressor.admit(
                isMic: true, text: "so the plan is to ship on Thursday morning", t0: 1_000, t1: 1_000))
        XCTAssertTrue(
            suppressor.admit(
                isMic: true, text: "so the plan is to ship on Thursday morning", t0: 2_000, t1: 2_000))
    }

    /// A long system line starts well outside the window but ends inside it.
    /// Keying candidates on the start time would let its echo through.
    func testALongSystemLineIsJudgedByWhenItFinished() {
        var suppressor = EchoSuppressor(windowMs: 8_000)
        XCTAssertTrue(
            suppressor.admit(
                isMic: false,
                text: "so the thing I wanted to raise about the migration deadline "
                    + "is that it has not moved at all since the last review",
                t0: 1_000,
                t1: 30_000))

        XCTAssertFalse(
            suppressor.admit(
                isMic: true,
                text: "the migration deadline is that it has not moved at all since "
                    + "the last review",
                t0: 31_000, t1: 34_000),
            "the echo of a line that finished a second ago should be caught")
    }
}
