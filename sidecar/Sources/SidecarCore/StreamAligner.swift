import Foundation

/// Keeps the mic and system streams in step for writing to one two-channel file.
///
/// The two capture paths are genuinely independent — different frameworks,
/// different callback rates, and either can stall (ScreenCaptureKit emits
/// nothing at all while the machine is silent). Writing whatever has arrived
/// from each would drift them apart, so the recording would slowly desync from
/// the transcript timestamps.
///
/// The rule here: time advances at a fixed rate, and a channel with nothing to
/// give contributes silence. Silence is the truthful value — that stream really
/// did produce no sound for that interval.
public struct StreamAligner {
    private var mic: [Float] = []
    private var system: [Float] = []

    public init() {}

    public var pendingMic: Int { mic.count }
    public var pendingSystem: Int { system.count }

    /// Frames available without padding either channel.
    public var alignedFrames: Int { min(mic.count, system.count) }

    public mutating func push(mic samples: [Float]) { mic.append(contentsOf: samples) }
    public mutating func push(system samples: [Float]) { system.append(contentsOf: samples) }

    /// Removes and returns `frames` from each channel, padding a short channel
    /// with silence. Returns nil when neither channel has anything at all —
    /// there is no honest output for a period where nothing was captured.
    public mutating func pull(frames: Int) -> (mic: [Float], system: [Float])? {
        precondition(frames > 0)
        guard !mic.isEmpty || !system.isEmpty else { return nil }

        let micOut = take(&mic, frames)
        let systemOut = take(&system, frames)
        return (micOut, systemOut)
    }

    /// Drains everything, padding the shorter channel so the file ends square.
    public mutating func drain() -> (mic: [Float], system: [Float])? {
        let frames = max(mic.count, system.count)
        guard frames > 0 else { return nil }
        return pull(frames: frames)
    }

    private func take(_ queue: inout [Float], _ frames: Int) -> [Float] {
        if queue.count >= frames {
            let out = Array(queue[0..<frames])
            queue.removeFirst(frames)
            return out
        }
        // Short: give what there is, pad the rest with silence.
        let out = queue + [Float](repeating: 0, count: frames - queue.count)
        queue.removeAll(keepingCapacity: true)
        return out
    }
}
