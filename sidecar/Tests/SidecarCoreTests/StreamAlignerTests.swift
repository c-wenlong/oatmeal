import XCTest

@testable import SidecarCore

final class StreamAlignerTests: XCTestCase {

    func testPullsEqualFramesFromBothChannels() {
        var aligner = StreamAligner()
        aligner.push(mic: [1, 2, 3, 4])
        aligner.push(system: [5, 6, 7, 8])

        let block = aligner.pull(frames: 4)
        XCTAssertEqual(block?.mic, [1, 2, 3, 4])
        XCTAssertEqual(block?.system, [5, 6, 7, 8])
        XCTAssertEqual(aligner.pendingMic, 0)
        XCTAssertEqual(aligner.pendingSystem, 0)
    }

    func testShortChannelIsPaddedWithSilenceNotDropped() {
        var aligner = StreamAligner()
        aligner.push(mic: [1, 2, 3, 4])
        aligner.push(system: [9])

        // ScreenCaptureKit genuinely emits nothing while the machine is silent.
        // Padding keeps the two channels time-aligned; taking only what both
        // have would let the mic run ahead and desync the recording.
        let block = aligner.pull(frames: 4)
        XCTAssertEqual(block?.mic, [1, 2, 3, 4])
        XCTAssertEqual(block?.system, [9, 0, 0, 0])
    }

    func testChannelsStayAlignedAcrossUnevenArrival() {
        var aligner = StreamAligner()
        // Mic arrives steadily, system in one late burst.
        aligner.push(mic: [1, 2])
        _ = aligner.pull(frames: 2)  // system silent for this block
        aligner.push(mic: [3, 4])
        aligner.push(system: [7, 8])
        let second = aligner.pull(frames: 2)

        // The burst must land in the block it arrived in, not be back-dated
        // into the earlier silent block.
        XCTAssertEqual(second?.mic, [3, 4])
        XCTAssertEqual(second?.system, [7, 8])
    }

    func testReturnsNilWhenNothingHasBeenCaptured() {
        var aligner = StreamAligner()
        // Emitting silence here would write phantom audio for a period where
        // capture simply wasn't running.
        XCTAssertNil(aligner.pull(frames: 512))
    }

    func testKeepsRemainderForTheNextPull() {
        var aligner = StreamAligner()
        aligner.push(mic: [1, 2, 3, 4, 5])
        aligner.push(system: [1, 2, 3, 4, 5])

        _ = aligner.pull(frames: 2)
        XCTAssertEqual(aligner.pendingMic, 3)

        let rest = aligner.pull(frames: 3)
        XCTAssertEqual(rest?.mic, [3, 4, 5])
    }

    func testAlignedFramesReportsTheUnpaddedOverlap() {
        var aligner = StreamAligner()
        aligner.push(mic: [1, 2, 3])
        aligner.push(system: [1])
        XCTAssertEqual(aligner.alignedFrames, 1)
    }

    func testDrainSquaresOffTheEndOfTheFile() {
        var aligner = StreamAligner()
        aligner.push(mic: [1, 2, 3])
        aligner.push(system: [9])

        let tail = aligner.drain()
        // Both channels must end at the same length or the file is malformed.
        XCTAssertEqual(tail?.mic.count, 3)
        XCTAssertEqual(tail?.system.count, 3)
        XCTAssertEqual(tail?.system, [9, 0, 0])
        XCTAssertNil(aligner.drain(), "drain should leave nothing behind")
    }

    func testDrainOfAnEmptyAlignerYieldsNothing() {
        var aligner = StreamAligner()
        XCTAssertNil(aligner.drain())
    }
}
