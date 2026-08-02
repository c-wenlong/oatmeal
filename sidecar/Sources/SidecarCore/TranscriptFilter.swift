import Foundation

/// Rejects the non-speech text Whisper emits when it has nothing real to
/// transcribe.
///
/// Second half of the G2 finding: even with a voice-activity gate in front, a
/// window containing a cough, music, or room tone comes back as a *sound
/// description* rather than speech — `[BLANK_AUDIO]`, `[Music]`,
/// `(upbeat music)`, `(audience applauding)`, `♪ ... ♪`. These are annotations,
/// not utterances. Persisting them would put invented content in the transcript
/// and hand the summarizer things nobody said.
public enum TranscriptFilter {

    /// Whisper's own bracket/paren annotations, plus common hallucinations on
    /// silence. Matched case-insensitively against the *entire* trimmed line, so
    /// a real sentence that merely contains a parenthetical is untouched.
    private static let annotationPatterns = [
        // Wholly bracketed or parenthesised: "[BLANK_AUDIO]", "(upbeat music)".
        "^\\[[^\\]]*\\]$",
        "^\\([^)]*\\)$",
        // Music-note wrapped lyrics: "♪ ... ♪".
        "^♪.*♪$",
        "^[♪♫\\s]+$",
        // Whisper sometimes emits bare asterisked stage directions.
        "^\\*[^*]*\\*$",
    ]

    /// Phrases Whisper repeats on long silences even without brackets. These are
    /// matched exactly (trimmed, case-insensitive) so ordinary speech that
    /// happens to contain them is kept.
    private static let hallucinationPhrases: Set<String> = [
        "thanks for watching!",
        "thank you for watching!",
        "thanks for watching",
        "you",
        "bye.",
        "bye",
        ".",
        "...",
    ]

    /// True when the text is an annotation or a known silence hallucination
    /// rather than something a person said.
    public static func isNonSpeech(_ text: String) -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return true }

        let lowered = trimmed.lowercased()
        if hallucinationPhrases.contains(lowered) { return true }

        for pattern in annotationPatterns {
            if trimmed.range(of: pattern, options: [.regularExpression, .caseInsensitive]) != nil {
                return true
            }
        }
        return false
    }

    /// Sound descriptions Whisper writes in parentheses. Matched by content
    /// rather than by the brackets alone, because a parenthetical can also be
    /// genuine speech — "we agreed (finally) to ship" must survive.
    private static let soundDescription =
        "\\((?:[^)]*\\b(?:music|applause|applauding|laughter|laughing|silence|"
        + "blank|inaudible|noise|coughs?|sighs?|clears throat|footsteps|beep\\w*)"
        + "[^)]*)\\)"

    /// Normalised text, or nil if it should never reach the transcript.
    ///
    /// Annotations are stripped wherever they appear, not just at the start.
    /// Whisper readily appends one to the end of real speech — a live run
    /// produced "...on Thursday morning. [BLANK_AUDIO]" — and a leading-only
    /// strip lets that straight into the transcript.
    public static func clean(_ text: String) -> String? {
        var working = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !isNonSpeech(working) else { return nil }

        // Square brackets are unambiguous: Whisper only uses them for
        // annotations, never for transcribed speech.
        working = working.replacingOccurrences(
            of: "\\[[^\\]]*\\]", with: " ", options: [.regularExpression])

        working = working.replacingOccurrences(
            of: soundDescription, with: " ",
            options: [.regularExpression, .caseInsensitive])

        working = working.replacingOccurrences(
            of: "[♪♫]", with: " ", options: [.regularExpression])

        // Collapse the whitespace the removals left behind.
        working = working.replacingOccurrences(
            of: "\\s+", with: " ", options: [.regularExpression])
        // ...and any space stranded before punctuation.
        working = working.replacingOccurrences(
            of: "\\s+([.,!?;:])", with: "$1", options: [.regularExpression])

        let result = working.trimmingCharacters(in: .whitespacesAndNewlines)
        return isNonSpeech(result) ? nil : result
    }
}
