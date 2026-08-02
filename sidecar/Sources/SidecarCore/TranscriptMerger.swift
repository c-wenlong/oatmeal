import Foundation

/// Stitches together transcripts from overlapping ASR windows.
///
/// Windows overlap by design — a word split across a boundary is transcribed
/// badly unless both sides see it whole. The cost is that the overlap region is
/// transcribed twice, so naive concatenation stutters:
///
///     window 1: "so the deadline for the migration"
///     window 2: "for the migration is the fourteenth"
///     naive:    "so the deadline for the migration for the migration is the fourteenth"
///
/// This finds the longest suffix of what we already have that matches a prefix
/// of the new window, and returns only what is genuinely new.
public enum TranscriptMerger {

    /// Words reduced to a comparable form. ASR punctuates and capitalises the
    /// same word differently across windows ("migration" vs "migration,"), so
    /// matching on raw text would miss most real overlaps.
    static func normalise(_ word: String) -> String {
        word.lowercased().trimmingCharacters(
            in: CharacterSet.alphanumerics.inverted)
    }

    static func words(_ text: String) -> [String] {
        text.split(whereSeparator: { $0 == " " || $0 == "\n" || $0 == "\t" })
            .map(String.init)
            .filter { !$0.isEmpty }
    }

    /// Number of trailing words of `previous` that also open `current`.
    ///
    /// Longest match wins: a short accidental match ("the") would otherwise
    /// truncate the new window at the wrong place.
    public static func overlapLength(previous: String, current: String) -> Int {
        let previousWords = words(previous).map(normalise).filter { !$0.isEmpty }
        let currentWords = words(current).map(normalise).filter { !$0.isEmpty }
        guard !previousWords.isEmpty, !currentWords.isEmpty else { return 0 }

        let maximum = min(previousWords.count, currentWords.count)
        var best = 0
        for length in stride(from: maximum, through: 1, by: -1) {
            let tail = previousWords.suffix(length)
            let head = currentWords.prefix(length)
            if Array(tail) == Array(head) {
                best = length
                break
            }
        }
        return best
    }

    /// The portion of `current` not already covered by `previous`.
    ///
    /// Returns nil when the new window adds nothing — which happens whenever
    /// someone stops talking and the trailing windows re-transcribe the same
    /// final sentence.
    public static func newText(previous: String, current: String) -> String? {
        let currentWords = words(current)
        guard !currentWords.isEmpty else { return nil }

        let previousTrimmed = previous.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !previousTrimmed.isEmpty else { return current }

        let overlap = overlapLength(previous: previousTrimmed, current: current)
        guard overlap < currentWords.count else { return nil }

        let remainder = currentWords.dropFirst(overlap).joined(separator: " ")
        let trimmed = remainder.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
