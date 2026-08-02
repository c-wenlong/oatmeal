import XCTest

@testable import SidecarCore

final class RingBufferTests: XCTestCase {

    func testStartsEmpty() {
        let ring = RingBuffer(capacity: 4)
        XCTAssertEqual(ring.count, 0)
        XCTAssertFalse(ring.isFull)
        XCTAssertTrue(ring.snapshot().isEmpty)
    }

    func testPartialFillReturnsOnlyWhatWasWritten() {
        var ring = RingBuffer(capacity: 8)
        ring.append([1, 2, 3])
        XCTAssertEqual(ring.count, 3)
        XCTAssertFalse(ring.isFull)
        // Must not report the zero-filled remainder as captured audio.
        XCTAssertEqual(ring.snapshot(), [1, 2, 3])
    }

    func testExactFill() {
        var ring = RingBuffer(capacity: 4)
        ring.append([1, 2, 3, 4])
        XCTAssertTrue(ring.isFull)
        XCTAssertEqual(ring.snapshot(), [1, 2, 3, 4])
    }

    func testWrapAroundKeepsTheNewestSamplesInOrder() {
        var ring = RingBuffer(capacity: 4)
        ring.append([1, 2, 3, 4])
        ring.append([5, 6])
        // Oldest-first ordering is what makes the pre-roll play back correctly;
        // a naive implementation returns [5, 6, 3, 4] here.
        XCTAssertEqual(ring.snapshot(), [3, 4, 5, 6])
        XCTAssertEqual(ring.count, 4)
    }

    func testManySmallAppendsWrapCorrectly() {
        var ring = RingBuffer(capacity: 3)
        for value in 1...10 {
            ring.append([Float(value)])
        }
        XCTAssertEqual(ring.snapshot(), [8, 9, 10])
    }

    func testChunkLargerThanCapacityKeepsItsTail() {
        var ring = RingBuffer(capacity: 3)
        ring.append([1, 2, 3, 4, 5, 6, 7])
        XCTAssertEqual(ring.snapshot(), [5, 6, 7])
        XCTAssertTrue(ring.isFull)
    }

    func testChunkExactlyCapacity() {
        var ring = RingBuffer(capacity: 3)
        ring.append([9, 9, 9])
        ring.append([1, 2, 3])
        XCTAssertEqual(ring.snapshot(), [1, 2, 3])
    }

    func testAppendingNothingIsANoOp() {
        var ring = RingBuffer(capacity: 3)
        ring.append([1])
        ring.append([])
        XCTAssertEqual(ring.snapshot(), [1])
    }

    func testRemoveAllResetsToEmptyNotToSilence() {
        var ring = RingBuffer(capacity: 3)
        ring.append([1, 2, 3])
        ring.removeAll()
        XCTAssertEqual(ring.count, 0)
        // A cleared buffer must yield nothing, not three zero samples that
        // would be written to the file as phantom silence.
        XCTAssertTrue(ring.snapshot().isEmpty)
    }

    func testSecondsInitSizesForTheSampleRate() {
        let ring = RingBuffer(seconds: 60, sampleRate: 16_000)
        XCTAssertEqual(ring.capacity, 960_000)
    }

    func testSixtySecondPreRollHoldsExactlySixtySeconds() {
        let rate = 16_000.0
        var ring = RingBuffer(seconds: 60, sampleRate: rate)
        // Two minutes of audio through a one-minute buffer.
        for second in 0..<120 {
            ring.append([Float](repeating: Float(second), count: Int(rate)))
        }
        XCTAssertEqual(ring.count, Int(60 * rate))
        // The oldest retained second must be minute two, second zero (=60).
        XCTAssertEqual(ring.snapshot().first, 60)
        XCTAssertEqual(ring.snapshot().last, 119)
    }
}
