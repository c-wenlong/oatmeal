import Foundation

/// Drops mic lines that are really the speakers bleeding back in.
///
/// The second G2 finding: played through laptop speakers rather than headphones,
/// the microphone picks up the far end and Whisper transcribes a garbled copy of
/// it. The same sentence then appears on *both* channels, and since `mic` = you
/// and `system` = them, that copy is attributed to the wrong person. Attribution
/// is the whole reason for running two streams, so a bleed-through is not a
/// cosmetic duplicate — it puts words in the user's mouth.
///
/// `MicCapture` enables Apple's voice-processing AEC, which removes most of it
/// in hardware. This is the second line of defence, for what survives and for
/// the machines where AEC cannot be turned on.
///
/// Comparison is on text rather than audio deliberately: by the time bleed has
/// been through a speaker, a room, a microphone and an ASR model, the waveforms
/// have nothing in common but the words do.
public struct EchoSuppressor {
    /// How far back a system line stays a candidate echo source.
    ///
    /// Generous, because the two streams settle their windows independently and
    /// a mic final can lag the system final that caused it by several seconds.
    public let windowMs: Int

    /// How much of the shorter line must appear in the longer one.
    public let threshold: Double

    /// Lines shorter than this are never suppressed.
    ///
    /// Short utterances are where genuine agreement lives — "yes", "sounds good",
    /// "the fourteenth" — and people really do repeat what was just said. With
    /// nothing but a handful of tokens there is no way to tell a repeat from an
    /// echo, and silently deleting the user's own words is far worse than
    /// keeping a duplicate.
    public let minimumTokens: Int

    /// Recent system lines, keyed by when they finished playing.
    private var recentSystem: [(tokens: Set<String>, endedAt: Int)] = []

    public init(windowMs: Int = 8_000, threshold: Double = 0.65, minimumTokens: Int = 4) {
        self.windowMs = windowMs
        self.threshold = threshold
        self.minimumTokens = minimumTokens
    }

    /// Records a settled system line as a possible echo source.
    ///
    /// Keyed on when it *finished* playing, because that is when the room stops
    /// producing it — the mic copy can only start before then, never after.
    public mutating func noteSystem(text: String, endedAt: Int) {
        let tokens = Self.tokens(text)
        guard !tokens.isEmpty else { return }
        recentSystem.append((tokens, endedAt))
        forget(before: endedAt - windowMs)
    }

    /// True when this mic line looks like the far end coming back through the room.
    public mutating func isEcho(micText text: String, t0: Int) -> Bool {
        forget(before: t0 - windowMs)

        let micTokens = Self.tokens(text)
        guard micTokens.count >= minimumTokens else { return false }

        return recentSystem.contains { candidate in
            candidate.tokens.count >= minimumTokens
                && Self.containment(micTokens, candidate.tokens) >= threshold
        }
    }

    /// Decides whether a settled line reaches the wire, and updates state.
    ///
    /// The routing rule lives here rather than at the call site so it is covered
    /// by tests: a system line is always kept *and* becomes a candidate echo
    /// source, a mic line is kept unless it matches one. Getting this backwards
    /// would suppress the far end and keep the bleed — the exact inversion of
    /// what is wanted, and invisible until someone reads a transcript.
    ///
    /// `isMic` rather than an `AudioSource`: this module deliberately has no
    /// dependency on the protocol package, so it stays testable without it.
    public mutating func admit(isMic: Bool, text: String, t0: Int, t1: Int) -> Bool {
        guard isMic else {
            noteSystem(text: text, endedAt: t1)
            return true
        }
        return !isEcho(micText: text, t0: t0)
    }

    private mutating func forget(before cutoff: Int) {
        recentSystem.removeAll { $0.endedAt < cutoff }
    }

    /// Overlap as a fraction of the *shorter* line.
    ///
    /// Containment rather than Jaccard: bleed usually arrives mangled and
    /// clipped, so the mic copy is a rough subset of what was actually said.
    /// Jaccard would punish it for the words the microphone lost and let the
    /// echo through.
    static func containment(_ a: Set<String>, _ b: Set<String>) -> Double {
        let smaller = min(a.count, b.count)
        guard smaller > 0 else { return 0 }
        return Double(a.intersection(b).count) / Double(smaller)
    }

    /// Lowercased words, punctuation removed.
    ///
    /// ASR punctuates the same sentence differently on each channel, so
    /// punctuation carries no signal about whether two lines are the same words.
    static func tokens(_ text: String) -> Set<String> {
        let cleaned = text.lowercased().map { character -> Character in
            character.isLetter || character.isNumber ? character : " "
        }
        return Set(String(cleaned).split(separator: " ").map(String.init))
    }
}
