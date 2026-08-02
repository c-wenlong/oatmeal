import XCTest

@testable import SidecarCore

/// These encode the G2 spike findings. If they ever go green while the
/// behaviour regresses, the transcript fills with content nobody said — and the
/// summarizer treats it as fact. See spike/FINDINGS.md.
final class TranscriptFilterTests: XCTestCase {

    /// Captured verbatim from the G2 spike run against a near-silent mic.
    private let observedHallucinations = [
        "(upbeat music)",
        "(audience applauding)",
        "[BLANK_AUDIO]",
        "[Music]",
        "(soft music)",
        "♪ Yeah, I lookin' to kill you with us ♪ ♪ I'm never, never, never ♪",
    ]

    func testRejectsEveryHallucinationTheSpikeActuallyProduced() {
        for text in observedHallucinations {
            XCTAssertTrue(
                TranscriptFilter.isNonSpeech(text),
                "would have persisted invented content: \(text)")
            XCTAssertNil(TranscriptFilter.clean(text))
        }
    }

    func testRejectsEmptyAndWhitespaceOnly() {
        XCTAssertTrue(TranscriptFilter.isNonSpeech(""))
        XCTAssertTrue(TranscriptFilter.isNonSpeech("   \n "))
    }

    func testRejectsKnownSilenceFillerPhrases() {
        // Whisper emits these on long silences with no brackets to give it away.
        XCTAssertTrue(TranscriptFilter.isNonSpeech("Thanks for watching!"))
        XCTAssertTrue(TranscriptFilter.isNonSpeech("you"))
        XCTAssertTrue(TranscriptFilter.isNonSpeech("..."))
    }

    func testKeepsRealSpeech() {
        let real = [
            "So the deadline for the migration is the fourteenth.",
            "Got it, I'll own the rollback plan.",
            "Perfect, let's review it on Thursday.",
        ]
        for text in real {
            XCTAssertFalse(TranscriptFilter.isNonSpeech(text), "dropped real speech: \(text)")
            XCTAssertEqual(TranscriptFilter.clean(text), text)
        }
    }

    func testKeepsSpeechThatMerelyContainsParentheses() {
        // Over-eager filtering is its own failure: this is a real sentence.
        let text = "We agreed (finally) to ship on Thursday."
        XCTAssertFalse(TranscriptFilter.isNonSpeech(text))
        XCTAssertEqual(TranscriptFilter.clean(text), text)
    }

    func testKeepsSpeechThatMentionsMusic() {
        let text = "The music at the venue was too loud."
        XCTAssertFalse(TranscriptFilter.isNonSpeech(text))
    }

    func testStripsATrailingAnnotationFromRealSpeech() {
        // Observed live: Whisper appended this to a correctly transcribed
        // sentence. A leading-only strip let it into the transcript verbatim.
        XCTAssertEqual(
            TranscriptFilter.clean(
                "Let us review the rollback plan on Thursday morning. [BLANK_AUDIO]"),
            "Let us review the rollback plan on Thursday morning.")
    }

    func testStripsAnAnnotationFromTheMiddleOfASentence() {
        XCTAssertEqual(
            TranscriptFilter.clean("The deadline [Music] is Thursday."),
            "The deadline is Thursday.")
    }

    func testStripsSoundDescriptionsButKeepsOrdinaryParentheticals() {
        // The distinguishing signal is the content, not the bracket type.
        XCTAssertEqual(
            TranscriptFilter.clean("(upbeat music) We shipped it."),
            "We shipped it.")
        XCTAssertEqual(
            TranscriptFilter.clean("We agreed (finally) to ship on Thursday."),
            "We agreed (finally) to ship on Thursday.")
    }

    func testFullyMusicNoteWrappedTextIsDroppedEntirely() {
        // Whisper wraps *lyrics* in music notes. Those are never meeting speech,
        // and the G2 hallucination had exactly this shape, so the whole line
        // goes — stripping the notes and keeping the words would smuggle a
        // song into the transcript.
        XCTAssertNil(TranscriptFilter.clean("♪ We shipped it ♪"))
        XCTAssertNil(
            TranscriptFilter.clean("♪ Yeah, I lookin' to kill you with us ♪"))
    }

    func testAStrayMusicNoteDoesNotCostUsTheSentence() {
        // Unbalanced notes appear mid-transcript; the speech around them is real.
        XCTAssertEqual(
            TranscriptFilter.clean("We shipped it ♪ on Thursday."),
            "We shipped it on Thursday.")
    }

    func testDoesNotLeaveStrandedWhitespaceOrPunctuation() {
        let cleaned = TranscriptFilter.clean("The deadline [Music] , is Thursday .")
        XCTAssertNotNil(cleaned)
        XCTAssertFalse(cleaned!.contains("  "), "double space left behind: \(cleaned!)")
        XCTAssertFalse(cleaned!.contains(" ."), "space before punctuation: \(cleaned!)")
    }

    func testAnnotationOnlyTextStillYieldsNothing() {
        // Stripping must not turn a pure annotation into an empty string that
        // then gets persisted as a blank utterance.
        XCTAssertNil(TranscriptFilter.clean("[BLANK_AUDIO]"))
        XCTAssertNil(TranscriptFilter.clean("[BLANK_AUDIO] [Music]"))
    }

    func testStripsALeadingAnnotationButKeepsTheSentence() {
        // Whisper often prefixes an annotation to genuine speech; dropping the
        // whole line would lose the sentence with it.
        XCTAssertEqual(
            TranscriptFilter.clean("[BLANK_AUDIO] So the deadline is Thursday."),
            "So the deadline is Thursday.")
        XCTAssertEqual(
            TranscriptFilter.clean("(upbeat music) Let's begin."),
            "Let's begin.")
    }

    func testTrimsSurroundingWhitespace() {
        XCTAssertEqual(TranscriptFilter.clean("  hello there  "), "hello there")
    }
}

final class VoiceActivityTests: XCTestCase {
    private let rate = 16_000.0

    private func silence(seconds: Double) -> [Float] {
        [Float](repeating: 0, count: Int(seconds * rate))
    }

    /// Low-level noise, like a quiet room. The spike's `rms > 0.001` gate let
    /// this through and Whisper hallucinated over it.
    private func roomTone(seconds: Double, amplitude: Float = 0.002) -> [Float] {
        (0..<Int(seconds * rate)).map { i in
            amplitude * Float(sin(Double(i) * 0.05))
        }
    }

    private func speech(seconds: Double, amplitude: Float = 0.3) -> [Float] {
        (0..<Int(seconds * rate)).map { i in
            amplitude * Float(sin(2.0 * Double.pi * 220.0 * Double(i) / rate))
        }
    }

    func testDigitalSilenceIsNotSpeech() {
        let vad = VoiceActivityDetector()
        XCTAssertFalse(vad.containsSpeech(silence(seconds: 5)))
        XCTAssertEqual(vad.speechSeconds(in: silence(seconds: 5)), 0)
    }

    func testQuietRoomToneIsNotSpeech() {
        // The exact regression from G2: this is what produced "(upbeat music)".
        let vad = VoiceActivityDetector()
        XCTAssertFalse(
            vad.containsSpeech(roomTone(seconds: 5)),
            "room tone passed the gate — Whisper will invent content for it")
    }

    func testSustainedSpeechPasses() {
        let vad = VoiceActivityDetector()
        XCTAssertTrue(vad.containsSpeech(speech(seconds: 2)))
    }

    func testAnIsolatedClickIsNotSpeech() {
        // A door closing or a key press is loud but brief. Duration is what
        // separates it from someone talking.
        let vad = VoiceActivityDetector()
        var window = silence(seconds: 5)
        for i in 0..<160 { window[i] = 0.9 }  // 10ms burst
        XCTAssertFalse(vad.containsSpeech(window))
    }

    func testSpeechInAMostlySilentWindowStillCounts() {
        // Someone answering "yes" after a long pause must not be discarded.
        let vad = VoiceActivityDetector()
        let window = silence(seconds: 4) + speech(seconds: 0.5) + silence(seconds: 4)
        XCTAssertTrue(vad.containsSpeech(window))
    }

    func testThresholdIsStricterThanTheSpikeGate() {
        // Guards against silently reverting to the spike's value.
        XCTAssertGreaterThan(VoiceActivityDetector().energyThreshold, 0.001)
    }

    func testEmptyInput() {
        XCTAssertFalse(VoiceActivityDetector().containsSpeech([]))
    }
}

final class TranscriptMergerTests: XCTestCase {

    func testReturnsOnlyTheNewWordsAcrossAnOverlap() {
        let previous = "so the deadline for the migration"
        let current = "for the migration is the fourteenth"
        XCTAssertEqual(
            TranscriptMerger.newText(previous: previous, current: current),
            "is the fourteenth")
    }

    func testMatchesDespitePunctuationAndCaseDifferences() {
        // ASR punctuates the same word differently between windows.
        let previous = "the deadline for the Migration"
        let current = "for the migration, is the fourteenth."
        XCTAssertEqual(
            TranscriptMerger.newText(previous: previous, current: current),
            "is the fourteenth.")
    }

    func testNoOverlapReturnsEverything() {
        XCTAssertEqual(
            TranscriptMerger.newText(previous: "completely different", current: "brand new text"),
            "brand new text")
    }

    func testAFullyDuplicateWindowAddsNothing() {
        // Trailing windows re-transcribe the same final sentence once the room
        // goes quiet; emitting it repeatedly would stutter the transcript.
        XCTAssertNil(
            TranscriptMerger.newText(
                previous: "is the fourteenth", current: "is the fourteenth"))
    }

    func testEmptyPreviousReturnsTheWholeWindow() {
        XCTAssertEqual(
            TranscriptMerger.newText(previous: "", current: "hello there"),
            "hello there")
    }

    func testEmptyCurrentYieldsNothing() {
        XCTAssertNil(TranscriptMerger.newText(previous: "hello", current: "   "))
    }

    func testPrefersTheLongestOverlapNotTheFirstMatch() {
        // "the" matches early; taking it would re-emit "deadline for the".
        let previous = "the deadline for the"
        let current = "the deadline for the fourteenth"
        XCTAssertEqual(
            TranscriptMerger.overlapLength(previous: previous, current: current), 4)
        XCTAssertEqual(
            TranscriptMerger.newText(previous: previous, current: current), "fourteenth")
    }

    func testOverlapLengthIsZeroWhenNothingMatches() {
        XCTAssertEqual(
            TranscriptMerger.overlapLength(previous: "alpha beta", current: "gamma delta"), 0)
    }

    func testHandlesAWindowThatRepeatsASingleWord() {
        XCTAssertEqual(
            TranscriptMerger.newText(previous: "yes yes", current: "yes yes yes"),
            "yes")
    }

    func testRealisticThreeWindowSequenceDoesNotStutter() {
        let windows = [
            "So the deadline for the",
            "the deadline for the migration is",
            "migration is the fourteenth.",
        ]
        var transcript = ""
        for window in windows {
            if let addition = TranscriptMerger.newText(previous: transcript, current: window) {
                transcript += transcript.isEmpty ? addition : " " + addition
            }
        }
        XCTAssertEqual(transcript, "So the deadline for the migration is the fourteenth.")
    }
}
