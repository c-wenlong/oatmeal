import XCTest

@testable import SidecarCore

final class WindowPolicyTests: XCTestCase {
    private let rate = 16_000.0

    private func samples(_ seconds: Double) -> Int { Int(seconds * 16_000) }

    func testWaitsWhileThereIsNothingWorthTranscribing() {
        let policy = WindowPolicy()
        XCTAssertEqual(
            policy.next(bufferedSamples: 0, secondsSinceLastPartial: 10), .wait)
    }

    func testEmitsAPartialOnceTheIntervalElapses() {
        let policy = WindowPolicy(partialIntervalSeconds: 2)
        XCTAssertEqual(
            policy.next(bufferedSamples: samples(1), secondsSinceLastPartial: 1.0), .wait)
        XCTAssertEqual(
            policy.next(bufferedSamples: samples(1), secondsSinceLastPartial: 2.0), .partial)
    }

    func testSettlesOnceTheWindowIsFull() {
        let policy = WindowPolicy(windowSeconds: 30, partialIntervalSeconds: 2)
        // Just under the window: still in-flight, never settled.
        XCTAssertEqual(
            policy.next(bufferedSamples: samples(29.9), secondsSinceLastPartial: 5), .partial)
        // A full window settles regardless of how recently a partial went out.
        XCTAssertEqual(
            policy.next(bufferedSamples: samples(30), secondsSinceLastPartial: 0), .settle)
    }

    func testSettlingBeatsAPendingPartial() {
        // No point emitting in-flight text for audio about to be finalised.
        let policy = WindowPolicy(windowSeconds: 30, partialIntervalSeconds: 2)
        XCTAssertEqual(
            policy.next(bufferedSamples: samples(35), secondsSinceLastPartial: 10), .settle)
    }

    func testRetainsTheOverlapTailNotTheWholeWindow() {
        let policy = WindowPolicy(windowSeconds: 30, overlapSeconds: 5)
        XCTAssertEqual(policy.retainedSamples(), samples(5))
        // Keeping the whole window would re-transcribe everything forever;
        // keeping nothing would clip words across the boundary.
        XCTAssertLessThan(policy.retainedSamples(), samples(30))
        XCTAssertGreaterThan(policy.retainedSamples(), 0)
    }

    func testFlushesAMeaningfulTailAtStop() {
        let policy = WindowPolicy(partialIntervalSeconds: 2)
        XCTAssertTrue(policy.shouldFlush(bufferedSamples: samples(3)))
        XCTAssertTrue(policy.shouldFlush(bufferedSamples: samples(1)))
    }

    func testDoesNotFlushAScrapOfAudioAtStop() {
        // Sub-second remnants are the tail of something already emitted.
        let policy = WindowPolicy(partialIntervalSeconds: 2)
        XCTAssertFalse(policy.shouldFlush(bufferedSamples: samples(0.2)))
        XCTAssertFalse(policy.shouldFlush(bufferedSamples: 0))
    }

    func testSecondsConversionMatchesTheSampleRate() {
        let policy = WindowPolicy(sampleRate: 16_000)
        XCTAssertEqual(policy.seconds(forSamples: 16_000), 1.0, accuracy: 0.0001)
        XCTAssertEqual(policy.seconds(forSamples: 8_000), 0.5, accuracy: 0.0001)
    }

    func testASteadyStreamAlternatesPartialsThenSettles() {
        // Walks a realistic minute: partials while filling, a settle at 30s,
        // and the buffer dropping back to the overlap tail afterwards.
        let policy = WindowPolicy(
            windowSeconds: 30, overlapSeconds: 5, partialIntervalSeconds: 2)
        var buffered = 0
        var settles = 0
        var partials = 0
        var sinceLastPartial = 0.0

        for _ in 0..<60 {
            buffered += samples(1)
            sinceLastPartial += 1
            switch policy.next(
                bufferedSamples: buffered, secondsSinceLastPartial: sinceLastPartial)
            {
            case .settle:
                settles += 1
                buffered = policy.retainedSamples()
                sinceLastPartial = 0
            case .partial:
                partials += 1
                sinceLastPartial = 0
            case .wait:
                break
            }
        }

        XCTAssertEqual(settles, 2, "a 60s stream with a 30s window should settle twice")
        XCTAssertGreaterThan(partials, 10, "live text should refresh throughout")
    }
}
