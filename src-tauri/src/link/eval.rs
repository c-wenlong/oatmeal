//! Measuring link quality.
//!
//! G17's acceptance is comparative: the layered linker has to beat the
//! timestamp-only baseline, or its complexity is not worth carrying. "Looks
//! better" is not an answer to that, so this module makes it a number.
//!
//! The fixture corpus below is synthetic but built from the failure shapes that
//! actually occur — a note typed a beat late, two lines equally close in time,
//! a note written up afterwards. It is *not* a substitute for the ten real
//! meetings the roadmap asks for; it is the harness those meetings get fed
//! through. `evaluate` takes any corpus, so pointing it at real data is a matter
//! of loading rows instead of literals.

use super::{LinkParams, NoteInput, UtteranceInput};
use crate::embed::HashEmbedder;

/// One labelled example: a note, a transcript, and the utterance a human says
/// it refers to.
pub struct Case {
    pub name: &'static str,
    pub note_text: &'static str,
    pub note_at_ms: Option<i64>,
    /// `(id, start_ms, text)`
    pub utterances: Vec<(i64, i64, &'static str)>,
    /// The correct answer.
    pub expected: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accuracy {
    pub correct: usize,
    pub total: usize,
    /// Cases where the linker produced nothing at all.
    pub missed: usize,
}

impl Accuracy {
    pub fn rate(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.correct as f32 / self.total as f32
        }
    }
}

fn to_inputs(case: &Case) -> (Vec<NoteInput>, Vec<UtteranceInput>) {
    let note = NoteInput {
        block_id: "b1".into(),
        first_typed_at_ms: case.note_at_ms,
        text: case.note_text.into(),
        embedding: Some(HashEmbedder::vector(case.note_text)),
    };
    let utterances = case
        .utterances
        .iter()
        .map(|(id, start, text)| UtteranceInput {
            id: *id,
            start_ms: *start,
            end_ms: start + 3_000,
            text: (*text).into(),
            embedding: Some(HashEmbedder::vector(text)),
        })
        .collect();
    (vec![note], utterances)
}

/// Runs a linker over the corpus and scores its *top* link against the label.
pub fn evaluate(
    cases: &[Case],
    params: &LinkParams,
    linker: fn(&[NoteInput], &[UtteranceInput], &LinkParams) -> Vec<super::Link>,
) -> Accuracy {
    let mut correct = 0;
    let mut missed = 0;

    for case in cases {
        let (notes, utterances) = to_inputs(case);
        let links = linker(&notes, &utterances, params);
        match links.first() {
            None => missed += 1,
            Some(link) if link.utterance_id == case.expected => correct += 1,
            Some(_) => {}
        }
    }

    Accuracy {
        correct,
        total: cases.len(),
        missed,
    }
}

/// The fixture corpus.
///
/// Each case is a shape that shows up in real note-taking. Several are winnable
/// by the clock alone — that is deliberate, so a regression that breaks the
/// temporal layer shows up as a drop rather than being masked by semantics.
pub fn corpus() -> Vec<Case> {
    vec![
        Case {
            name: "note typed right after the line",
            note_text: "deadline 14th",
            note_at_ms: Some(21_000),
            utterances: vec![
                (1, 5_000, "morning everyone, let's get started"),
                (
                    2,
                    19_000,
                    "the deadline for the migration is the fourteenth",
                ),
            ],
            expected: 2,
        },
        Case {
            name: "tie in time, broken by topic",
            note_text: "deadline migration",
            note_at_ms: Some(20_000),
            utterances: vec![
                (1, 19_000, "who is bringing lunch tomorrow"),
                (2, 19_000, "the deadline for the migration is thursday"),
            ],
            expected: 2,
        },
        Case {
            name: "note typed a beat late, nearest line is small talk",
            note_text: "rollback plan owner",
            note_at_ms: Some(30_000),
            utterances: vec![
                (1, 12_000, "I'll own the rollback plan myself"),
                (2, 29_000, "anyway, moving on"),
            ],
            expected: 1,
        },
        Case {
            name: "note written up after the meeting",
            note_text: "deadline migration thursday",
            note_at_ms: None,
            utterances: vec![
                (1, 1_000, "the deadline for the migration is thursday"),
                (2, 60_000, "who is bringing lunch"),
            ],
            expected: 1,
        },
        Case {
            name: "several lines, only one on topic",
            note_text: "budget approval",
            note_at_ms: Some(40_000),
            utterances: vec![
                (1, 10_000, "the budget approval came through yesterday"),
                (2, 30_000, "let's talk about hiring"),
                (3, 38_000, "and the office move"),
            ],
            expected: 1,
        },
        Case {
            name: "immediate note, obvious answer",
            note_text: "ship thursday",
            note_at_ms: Some(12_000),
            utterances: vec![
                (1, 3_000, "unrelated preamble"),
                (2, 11_000, "we ship on thursday then"),
            ],
            expected: 2,
        },
        Case {
            name: "terse note, verbose line",
            note_text: "hiring freeze",
            note_at_ms: Some(25_000),
            utterances: vec![
                (1, 8_000, "so about the roadmap"),
                (
                    2,
                    22_000,
                    "we are putting a hiring freeze in place until the next quarter",
                ),
            ],
            expected: 2,
        },
        Case {
            name: "note about the earlier of two nearby lines",
            note_text: "security review blocker",
            note_at_ms: Some(35_000),
            utterances: vec![
                (1, 20_000, "the security review is the blocker right now"),
                (2, 33_000, "ok noted"),
            ],
            expected: 1,
        },
        Case {
            name: "long gap, still in window",
            note_text: "contract renewal",
            note_at_ms: Some(50_000),
            utterances: vec![
                (1, 12_000, "the contract renewal lands in march"),
                (2, 48_000, "right"),
            ],
            expected: 1,
        },
        Case {
            name: "note immediately after a decision",
            note_text: "agreed no rewrite",
            note_at_ms: Some(18_000),
            utterances: vec![
                (1, 6_000, "should we rewrite it"),
                (2, 16_000, "agreed, no rewrite for now"),
            ],
            expected: 2,
        },
        // ------------------------------------------------------------------
        // Cases where semantics cannot help and only the clock can.
        //
        // Without these the corpus is biased: every answer is the topically
        // matching line, so a pure-semantic weighting scores perfectly and the
        // sweep recommends throwing the temporal layer away. Real notes are
        // frequently shorthand that shares no vocabulary with what was said.
        Case {
            name: "shorthand note sharing no words with the line",
            note_text: "!!",
            note_at_ms: Some(14_000),
            utterances: vec![
                (1, 2_000, "let's start with the roadmap"),
                (2, 12_000, "we are cutting the release in half"),
            ],
            expected: 2,
        },
        Case {
            name: "one-word reaction",
            note_text: "important",
            note_at_ms: Some(26_000),
            utterances: vec![
                (1, 9_000, "the vendor wants a two year commitment"),
                (2, 24_000, "and they will not budge on the price"),
            ],
            expected: 2,
        },
        Case {
            name: "personal shorthand nobody said aloud",
            note_text: "ask J re: timeline",
            note_at_ms: Some(33_000),
            utterances: vec![
                (1, 5_000, "the design review is next week"),
                (2, 31_000, "we still do not have a date for the handover"),
            ],
            expected: 2,
        },
        Case {
            name: "abbreviation the transcript spells out",
            note_text: "q3 num",
            note_at_ms: Some(20_000),
            utterances: vec![
                (1, 4_000, "welcome back everyone"),
                (2, 18_000, "third quarter came in under what we forecast"),
            ],
            expected: 2,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{link_baseline, link_layered};

    #[test]
    fn the_corpus_is_labelled_consistently() {
        for case in corpus() {
            assert!(
                case.utterances
                    .iter()
                    .any(|(id, _, _)| *id == case.expected),
                "case '{}' expects an utterance that is not in its transcript",
                case.name
            );
        }
    }

    #[test]
    fn the_layered_linker_beats_the_timestamp_only_baseline() {
        // G17's acceptance, made measurable. The baseline exists precisely so
        // this claim is a number rather than an impression.
        let params = LinkParams::default();
        let cases = corpus();

        let baseline = evaluate(&cases, &params, link_baseline);
        let layered = evaluate(&cases, &params, link_layered);

        eprintln!(
            "baseline {}/{} ({:.0}%)  |  layered {}/{} ({:.0}%)",
            baseline.correct,
            baseline.total,
            baseline.rate() * 100.0,
            layered.correct,
            layered.total,
            layered.rate() * 100.0
        );

        assert!(
            layered.correct > baseline.correct,
            "the semantic layer did not beat the baseline ({} vs {}) — it is not \
             earning its complexity",
            layered.correct,
            baseline.correct
        );
    }

    #[test]
    fn the_baseline_still_gets_the_easy_cases_right() {
        // If this collapses, the temporal layer has regressed and the semantic
        // layer is quietly carrying everything.
        let accuracy = evaluate(&corpus(), &LinkParams::default(), link_baseline);
        assert!(
            accuracy.rate() >= 0.4,
            "baseline fell to {:.0}% — the temporal layer looks broken",
            accuracy.rate() * 100.0
        );
    }

    #[test]
    fn the_layered_linker_leaves_almost_nothing_unlinked() {
        let accuracy = evaluate(&corpus(), &LinkParams::default(), link_layered);
        assert!(
            accuracy.missed <= 1,
            "{} notes got no link at all",
            accuracy.missed
        );
    }

    /// Prints the whole weighting curve. Kept (ignored by default) because
    /// picking weights off a single peak is how you overfit a small corpus —
    /// the shape matters more than the maximum.
    ///
    /// `cargo test -- --ignored --nocapture weighting_curve`
    #[test]
    #[ignore]
    fn weighting_curve() {
        let cases = corpus();
        for step in 0..=10 {
            let alpha = step as f32 / 10.0;
            let params = LinkParams {
                alpha,
                beta: 1.0 - alpha,
                ..Default::default()
            };
            let accuracy = evaluate(&cases, &params, link_layered);
            eprintln!(
                "alpha={alpha:.1} beta={:.1}  {}/{}",
                1.0 - alpha,
                accuracy.correct,
                accuracy.total
            );
        }
    }

    #[test]
    fn the_chosen_weights_are_at_or_near_the_measured_optimum() {
        // Guards against someone "tidying" alpha/beta to round numbers that
        // happen to be worse. Sweeps the weighting and checks the shipped
        // default is within one case of the best score found.
        let cases = corpus();
        let mut best = 0usize;
        let mut best_alpha = 0.0f32;

        for step in 0..=10 {
            let alpha = step as f32 / 10.0;
            let params = LinkParams {
                alpha,
                beta: 1.0 - alpha,
                ..Default::default()
            };
            let accuracy = evaluate(&cases, &params, link_layered);
            if accuracy.correct > best {
                best = accuracy.correct;
                best_alpha = alpha;
            }
        }

        let shipped = evaluate(&cases, &LinkParams::default(), link_layered);
        eprintln!(
            "sweep best {best}/{} at alpha={best_alpha:.1}; shipped {}/{} at alpha={:.1}",
            cases.len(),
            shipped.correct,
            cases.len(),
            LinkParams::default().alpha
        );

        assert!(
            shipped.correct + 1 >= best,
            "shipped weights score {}/{} but {best}/{} is achievable at alpha={best_alpha:.1}",
            shipped.correct,
            cases.len(),
            cases.len()
        );
    }
}
