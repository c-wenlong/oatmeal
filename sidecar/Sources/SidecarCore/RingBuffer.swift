import Foundation

/// Fixed-capacity circular buffer of audio samples, newest-wins.
///
/// This is what makes "start recording" retroactive: capture runs into the ring
/// the whole time a meeting is armed, so pressing record recovers the ~60s that
/// already happened — the sentence someone was midway through when you realised
/// you wanted it.
///
/// Not thread-safe by itself; callers serialise access (the capture engine owns
/// one per stream and only touches it from its own queue).
public struct RingBuffer {
    private var storage: [Float]
    /// Index of the next write.
    private var head: Int = 0
    /// Total samples ever written, so we can tell "partially filled" from "wrapped".
    private var written: Int = 0

    public let capacity: Int

    public init(capacity: Int) {
        precondition(capacity > 0, "ring buffer needs a positive capacity")
        self.capacity = capacity
        self.storage = [Float](repeating: 0, count: capacity)
    }

    /// Samples currently retrievable.
    public var count: Int { min(written, capacity) }

    public var isFull: Bool { written >= capacity }

    public mutating func append(_ samples: [Float]) {
        guard !samples.isEmpty else { return }

        // A chunk larger than the ring can only leave its own tail behind;
        // copying the earlier part would be pure waste.
        if samples.count >= capacity {
            let tail = samples.suffix(capacity)
            storage = Array(tail)
            head = 0
            written += samples.count
            return
        }

        for sample in samples {
            storage[head] = sample
            head = (head + 1) % capacity
        }
        written += samples.count
    }

    /// Everything held, oldest first.
    public func snapshot() -> [Float] {
        if !isFull {
            return Array(storage[0..<head])
        }
        // Wrapped: the oldest sample sits at `head`.
        return Array(storage[head..<capacity]) + Array(storage[0..<head])
    }

    public mutating func removeAll() {
        head = 0
        written = 0
        storage = [Float](repeating: 0, count: capacity)
    }
}

extension RingBuffer {
    /// Convenience for sizing by duration.
    public init(seconds: Double, sampleRate: Double) {
        self.init(capacity: max(1, Int(seconds * sampleRate)))
    }
}
