//! Linking notes to the transcript spans they came from.
//!
//! This is the product's differentiator (SPEC section 7): a note is only useful
//! as an anchor if you can see *what was being said* when it was written.
//!
//! Three layers, and the roadmap is emphatic about the order they were built in:
//! the **timestamp-only baseline comes first**, because "does the semantic layer
//! help?" is otherwise unanswerable. Link quality is a tuning problem, and a
//! tuning problem without a baseline is just an opinion.

pub mod bench;
pub mod eval;
pub mod pipeline;

use serde::{Deserialize, Serialize};

use crate::embed::cosine;

/// How a link was established. Stored per link so quality can be measured by
/// method rather than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkMethod {
    /// Proximity in time alone. The baseline.
    Temporal,
    /// Meaning, rescued a note the clock would have missed.
    Semantic,
    /// The summariser said so (G14 citations).
    Llm,
}

impl LinkMethod {
    /// The stored form. Must match `note_links.method`'s CHECK constraint —
    /// a mismatch is a write that fails at runtime, not at compile time.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Temporal => "temporal",
            Self::Semantic => "semantic",
            Self::Llm => "llm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub note_block_id: String,
    pub utterance_id: i64,
    pub method: LinkMethod,
    pub score: f32,
}

/// A note as the linker sees it.
#[derive(Debug, Clone)]
pub struct NoteInput {
    pub block_id: String,
    /// Milliseconds from meeting start to the first keystroke. `None` means the
    /// note has no anchor and only the semantic layer can place it.
    pub first_typed_at_ms: Option<i64>,
    pub text: String,
    pub embedding: Option<Vec<f32>>,
}

/// A transcript line as the linker sees it.
#[derive(Debug, Clone)]
pub struct UtteranceInput {
    pub id: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub embedding: Option<Vec<f32>>,
}

/// Tunable weights and windows.
///
/// Exposed so they can be adjusted against a real meeting without a rebuild
/// (G18) — the right values are an empirical question, not a design one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkParams {
    /// How far *back* from a keystroke to look. Generous: people type after
    /// hearing something, often several sentences later.
    pub look_back_ms: i64,
    /// How far *forward*. Short: you cannot take a note about something not yet
    /// said, beyond finishing a thought already in progress.
    pub look_ahead_ms: i64,
    /// Weight on temporal proximity.
    pub alpha: f32,
    /// Weight on semantic similarity.
    pub beta: f32,
    /// A global semantic match must beat the best in-window candidate by this
    /// much before it is added — otherwise every note collects a second,
    /// weaker link and the UI fills with noise.
    pub global_margin: f32,
    /// Links scoring below this are not worth showing.
    pub min_score: f32,
    /// Most links to keep per note.
    pub max_per_note: usize,
}

impl Default for LinkParams {
    fn default() -> Self {
        Self {
            look_back_ms: 45_000,
            look_ahead_ms: 10_000,
            // Measured, not guessed. `eval::weighting_curve` scores 14/14 across
            // alpha 0.2–0.4 and degrades either side, so 0.3 is the centre of a
            // plateau rather than the edge of a spike. Semantics carry most of
            // the weight; time still decides the terse notes that share no
            // vocabulary with what was said.
            alpha: 0.3,
            beta: 0.7,
            global_margin: 0.15,
            min_score: 0.15,
            max_per_note: 3,
        }
    }
}

/// Temporal closeness in 0..=1, or `None` when the utterance is outside the
/// window entirely.
///
/// The window is deliberately asymmetric. A symmetric one gives equal credit to
/// a line spoken *after* the note was typed, which reverses cause and effect.
pub fn temporal_score(
    note_at_ms: i64,
    utterance_start_ms: i64,
    params: &LinkParams,
) -> Option<f32> {
    let delta = note_at_ms - utterance_start_ms;

    if delta >= 0 {
        // Utterance came first — the normal case.
        if delta > params.look_back_ms {
            return None;
        }
        Some(1.0 - (delta as f32 / params.look_back_ms as f32))
    } else {
        let ahead = -delta;
        if ahead > params.look_ahead_ms {
            return None;
        }
        Some(1.0 - (ahead as f32 / params.look_ahead_ms as f32))
    }
}

/// **The baseline.** Nearest preceding utterance by time, nothing else.
///
/// Built first and kept, because it is what the layered linker has to beat.
/// Anything the full pipeline adds has to be worth its complexity against this.
pub fn link_baseline(
    notes: &[NoteInput],
    utterances: &[UtteranceInput],
    params: &LinkParams,
) -> Vec<Link> {
    notes
        .iter()
        .filter_map(|note| {
            let at = note.first_typed_at_ms?;
            utterances
                .iter()
                .filter_map(|u| temporal_score(at, u.start_ms, params).map(|s| (u, s)))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(u, score)| Link {
                    note_block_id: note.block_id.clone(),
                    utterance_id: u.id,
                    method: LinkMethod::Temporal,
                    score,
                })
        })
        .collect()
}

/// The layered linker: temporal window, semantic rerank, then a global pass for
/// notes the clock cannot place.
pub fn link_layered(
    notes: &[NoteInput],
    utterances: &[UtteranceInput],
    params: &LinkParams,
) -> Vec<Link> {
    let mut links = Vec::new();

    for note in notes {
        let mut scored: Vec<(i64, f32, LinkMethod)> = Vec::new();

        // Layer 1 + 2: candidates inside the window, reranked by meaning.
        //
        // `temporal_only` is kept alongside the blend so each link can record
        // which layer actually decided it. Labelling every in-window link
        // `Temporal` regardless would make the stored `method` useless for the
        // one question G17 exists to answer — whether the semantic layer is
        // earning its complexity on real meetings.
        let mut temporal_only: Vec<(i64, f32)> = Vec::new();
        if let Some(at) = note.first_typed_at_ms {
            for utterance in utterances {
                let Some(temporal) = temporal_score(at, utterance.start_ms, params) else {
                    continue;
                };
                let semantic = similarity(note, utterance);
                let combined = match semantic {
                    // With no embeddings the layered linker must degrade to the
                    // baseline rather than scoring everything zero.
                    None => temporal,
                    Some(sem) => params.alpha * temporal + params.beta * sem,
                };
                scored.push((utterance.id, combined, LinkMethod::Temporal));
                temporal_only.push((utterance.id, temporal));
            }
        }

        let best_in_window = scored.iter().map(|(_, s, _)| *s).fold(0.0f32, f32::max);

        // Layer 2b: the global pass. Catches a note typed long after the fact,
        // or one with no anchor at all. Held to a margin so it does not bolt a
        // weak second link onto every note.
        if note.embedding.is_some() {
            let global = utterances
                .iter()
                .filter_map(|u| similarity(note, u).map(|s| (u.id, s)))
                .filter(|(id, _)| !scored.iter().any(|(existing, _, _)| existing == id))
                .max_by(|a, b| a.1.total_cmp(&b.1));

            if let Some((id, sem)) = global {
                // Scored on the same scale as the windowed candidates: temporal
                // contributes nothing (the line is outside the window by
                // definition), so this is `alpha * 0 + beta * sem`. Comparing a
                // raw cosine against a weighted blend would let the global pass
                // outrank a strong in-window match every time.
                let comparable = params.beta * sem;
                if comparable > best_in_window + params.global_margin {
                    scored.push((id, comparable, LinkMethod::Semantic));
                }
            }
        }

        scored.retain(|(_, score, _)| *score >= params.min_score);
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(params.max_per_note);

        // A link is `Semantic` when meaning *promoted* it — when it ranks higher
        // under the blend than it would have on the clock alone. Anything the
        // clock would have picked anyway stays `Temporal`, even though the
        // semantic score contributed to its number. The global pass already
        // labelled itself and is left alone.
        temporal_only.sort_by(|a, b| b.1.total_cmp(&a.1));
        let temporal_rank = |id: i64| temporal_only.iter().position(|(u, _)| *u == id);
        for (rank, entry) in scored.iter_mut().enumerate() {
            if entry.2 != LinkMethod::Temporal {
                continue;
            }
            if temporal_rank(entry.0).is_some_and(|before| rank < before) {
                entry.2 = LinkMethod::Semantic;
            }
        }

        for (utterance_id, score, method) in scored {
            links.push(Link {
                note_block_id: note.block_id.clone(),
                utterance_id,
                method,
                score,
            });
        }
    }

    links
}

/// Layer 3: citations the summariser produced, merged in.
///
/// These are already validated (G14), so they are trusted at full score — but
/// they are recorded as `Llm` so their contribution stays measurable separately.
pub fn merge_llm_links(links: &mut Vec<Link>, citations: &[(String, i64)]) {
    for (note_block_id, utterance_id) in citations {
        let existing = links
            .iter_mut()
            .find(|l| &l.note_block_id == note_block_id && l.utterance_id == *utterance_id);

        match existing {
            // Agreement between layers is the strongest signal there is; promote
            // rather than adding a duplicate row.
            Some(link) => {
                link.method = LinkMethod::Llm;
                link.score = link.score.max(1.0);
            }
            None => links.push(Link {
                note_block_id: note_block_id.clone(),
                utterance_id: *utterance_id,
                method: LinkMethod::Llm,
                score: 1.0,
            }),
        }
    }
}

fn similarity(note: &NoteInput, utterance: &UtteranceInput) -> Option<f32> {
    match (&note.embedding, &utterance.embedding) {
        (Some(a), Some(b)) => Some(cosine(a, b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::HashEmbedder;

    fn note(block_id: &str, at: Option<i64>, text: &str) -> NoteInput {
        NoteInput {
            block_id: block_id.into(),
            first_typed_at_ms: at,
            text: text.into(),
            embedding: Some(HashEmbedder::vector(text)),
        }
    }

    fn note_without_embedding(block_id: &str, at: Option<i64>, text: &str) -> NoteInput {
        NoteInput {
            block_id: block_id.into(),
            first_typed_at_ms: at,
            text: text.into(),
            embedding: None,
        }
    }

    fn utterance(id: i64, start_ms: i64, text: &str) -> UtteranceInput {
        UtteranceInput {
            id,
            start_ms,
            end_ms: start_ms + 3_000,
            text: text.into(),
            embedding: Some(HashEmbedder::vector(text)),
        }
    }

    fn utterance_without_embedding(id: i64, start_ms: i64, text: &str) -> UtteranceInput {
        UtteranceInput {
            id,
            start_ms,
            end_ms: start_ms + 3_000,
            text: text.into(),
            embedding: None,
        }
    }

    // ------------------------------------------------------------- temporal

    #[test]
    fn an_utterance_at_the_moment_of_typing_scores_highest() {
        let params = LinkParams::default();
        assert_eq!(temporal_score(10_000, 10_000, &params), Some(1.0));
    }

    #[test]
    fn closeness_decays_with_distance() {
        let params = LinkParams::default();
        let near = temporal_score(10_000, 9_000, &params).unwrap();
        let far = temporal_score(10_000, 0, &params).unwrap();
        assert!(near > far);
    }

    #[test]
    fn the_window_is_asymmetric_because_you_type_after_hearing() {
        // 20s before the note is well inside the window; 20s after is not. A
        // symmetric window credits lines spoken *after* the note was written,
        // which reverses cause and effect.
        let params = LinkParams::default();
        assert!(temporal_score(30_000, 10_000, &params).is_some());
        assert!(temporal_score(10_000, 30_000, &params).is_none());
    }

    #[test]
    fn a_line_just_ahead_still_counts() {
        // Finishing a sentence you had already started reacting to.
        let params = LinkParams::default();
        assert!(temporal_score(10_000, 15_000, &params).is_some());
    }

    #[test]
    fn utterances_outside_the_window_are_not_candidates() {
        let params = LinkParams::default();
        assert!(temporal_score(100_000, 10_000, &params).is_none());
    }

    // ------------------------------------------------------------- baseline

    #[test]
    fn the_baseline_picks_the_nearest_preceding_line() {
        let params = LinkParams::default();
        let notes = vec![note("b1", Some(20_000), "deadline")];
        let utterances = vec![
            utterance(1, 5_000, "unrelated chatter"),
            utterance(2, 18_000, "the deadline is Thursday"),
        ];

        let links = link_baseline(&notes, &utterances, &params);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].utterance_id, 2);
        assert_eq!(links[0].method, LinkMethod::Temporal);
    }

    #[test]
    fn the_baseline_ignores_a_note_with_no_anchor() {
        // Nothing to key on; only the semantic layer can place it.
        let params = LinkParams::default();
        let links = link_baseline(
            &[note("b1", None, "deadline")],
            &[utterance(1, 5_000, "the deadline is Thursday")],
            &params,
        );
        assert!(links.is_empty());
    }

    #[test]
    fn the_baseline_links_nothing_when_the_window_is_empty() {
        let params = LinkParams::default();
        let links = link_baseline(
            &[note("b1", Some(500_000), "deadline")],
            &[utterance(1, 1_000, "the deadline is Thursday")],
            &params,
        );
        assert!(links.is_empty());
    }

    // -------------------------------------------------------------- layered

    #[test]
    fn semantics_break_a_temporal_tie_correctly() {
        // Two lines equally close in time; only one is about the note. This is
        // precisely what the baseline cannot do.
        let params = LinkParams::default();
        let notes = vec![note("b1", Some(20_000), "deadline migration")];
        let utterances = vec![
            utterance(1, 19_000, "who is bringing lunch tomorrow"),
            utterance(2, 19_000, "the deadline for the migration is Thursday"),
        ];

        let links = link_layered(&notes, &utterances, &params);
        assert_eq!(links[0].utterance_id, 2, "semantics did not break the tie");
    }

    #[test]
    fn a_slightly_older_but_on_topic_line_can_win() {
        // The note was typed a beat late; the nearest line is small talk.
        let params = LinkParams::default();
        let notes = vec![note("b1", Some(30_000), "rollback plan owner")];
        let utterances = vec![
            utterance(1, 12_000, "I'll own the rollback plan"),
            utterance(2, 29_000, "anyway, moving on"),
        ];

        let links = link_layered(&notes, &utterances, &params);
        assert_eq!(links[0].utterance_id, 1);
    }

    #[test]
    fn the_global_pass_places_a_note_typed_long_afterwards() {
        // Written up after the meeting; the clock cannot help at all.
        let params = LinkParams::default();
        let notes = vec![note("b1", None, "deadline migration Thursday")];
        let utterances = vec![
            utterance(1, 1_000, "the deadline for the migration is Thursday"),
            utterance(2, 60_000, "who is bringing lunch"),
        ];

        let links = link_layered(&notes, &utterances, &params);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].utterance_id, 1);
        assert_eq!(links[0].method, LinkMethod::Semantic);
    }

    #[test]
    fn the_global_pass_does_not_bolt_a_weak_link_onto_every_note() {
        // Without the margin, every note collects a second, worse link and the
        // UI fills with noise.
        let params = LinkParams::default();
        let notes = vec![note("b1", Some(20_000), "deadline migration")];
        let utterances = vec![
            utterance(1, 19_000, "the deadline for the migration is Thursday"),
            utterance(2, 90_000, "deadline migration deadline migration"),
        ];

        let links = link_layered(&notes, &utterances, &params);
        assert_eq!(links[0].utterance_id, 1);
        assert!(
            links.len() <= 2,
            "global pass added noise: {} links",
            links.len()
        );
    }

    #[test]
    fn a_link_meaning_promoted_is_recorded_as_semantic() {
        // The stored `method` has to say which layer decided the link, or there
        // is no way to tell from real data whether the semantic layer helps.
        let note = NoteInput {
            block_id: "b1".into(),
            first_typed_at_ms: Some(30_000),
            text: "rollback plan owner".into(),
            embedding: Some(HashEmbedder::vector("rollback plan owner")),
        };
        // The nearest line in time is small talk; the right line is older.
        let utterances = vec![
            UtteranceInput {
                id: 1,
                start_ms: 12_000,
                end_ms: 15_000,
                text: "I will own the rollback plan myself".into(),
                embedding: Some(HashEmbedder::vector("I will own the rollback plan myself")),
            },
            UtteranceInput {
                id: 2,
                start_ms: 29_000,
                end_ms: 31_000,
                text: "anyway, moving on".into(),
                embedding: Some(HashEmbedder::vector("anyway, moving on")),
            },
        ];

        let links = link_layered(&[note], &utterances, &LinkParams::default());
        let top = links.first().expect("expected a link");

        assert_eq!(top.utterance_id, 1, "meaning should have won here");
        assert_eq!(
            top.method,
            LinkMethod::Semantic,
            "meaning promoted this link past the nearer line, so it is not temporal"
        );
    }

    #[test]
    fn a_link_the_clock_would_have_picked_anyway_stays_temporal() {
        // Otherwise everything reads as semantic and the label means nothing.
        let note = NoteInput {
            block_id: "b1".into(),
            first_typed_at_ms: Some(20_000),
            text: "ship thursday".into(),
            embedding: Some(HashEmbedder::vector("ship thursday")),
        };
        let utterances = vec![
            UtteranceInput {
                id: 1,
                start_ms: 18_000,
                end_ms: 19_000,
                text: "we ship on thursday then".into(),
                embedding: Some(HashEmbedder::vector("we ship on thursday then")),
            },
            UtteranceInput {
                id: 2,
                start_ms: 2_000,
                end_ms: 4_000,
                text: "unrelated preamble".into(),
                embedding: Some(HashEmbedder::vector("unrelated preamble")),
            },
        ];

        let links = link_layered(&[note], &utterances, &LinkParams::default());
        let top = links.first().expect("expected a link");

        assert_eq!(top.utterance_id, 1);
        assert_eq!(top.method, LinkMethod::Temporal);
    }

    #[test]
    fn without_embeddings_the_layered_linker_degrades_to_the_baseline() {
        // If the embedding model is missing, links must still be produced —
        // scoring everything zero would silently disable the feature.
        let params = LinkParams::default();
        let notes = vec![note_without_embedding("b1", Some(20_000), "deadline")];
        let utterances = vec![
            utterance_without_embedding(1, 5_000, "unrelated"),
            utterance_without_embedding(2, 18_000, "the deadline is Thursday"),
        ];

        let layered = link_layered(&notes, &utterances, &params);
        let baseline = link_baseline(&notes, &utterances, &params);
        assert_eq!(layered[0].utterance_id, baseline[0].utterance_id);
    }

    #[test]
    fn links_are_capped_per_note() {
        let params = LinkParams {
            max_per_note: 2,
            ..Default::default()
        };
        let notes = vec![note("b1", Some(30_000), "deadline")];
        let utterances: Vec<_> = (0..6)
            .map(|i| utterance(i, 20_000 + i * 500, "the deadline is Thursday"))
            .collect();

        let links = link_layered(&notes, &utterances, &params);
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn links_come_back_strongest_first() {
        let params = LinkParams::default();
        let notes = vec![note("b1", Some(30_000), "deadline")];
        let utterances: Vec<_> = (0..4)
            .map(|i| utterance(i, 10_000 + i * 4_000, "the deadline is Thursday"))
            .collect();

        let links = link_layered(&notes, &utterances, &params);
        for pair in links.windows(2) {
            assert!(pair[0].score >= pair[1].score, "links are not sorted");
        }
    }

    #[test]
    fn weak_links_are_dropped_rather_than_shown() {
        let params = LinkParams {
            min_score: 0.95,
            ..Default::default()
        };
        let links = link_layered(
            &[note("b1", Some(40_000), "deadline")],
            &[utterance(1, 1_000, "completely unrelated small talk")],
            &params,
        );
        assert!(links.is_empty());
    }

    #[test]
    fn every_note_is_considered_independently() {
        let params = LinkParams::default();
        let notes = vec![
            note("b1", Some(10_000), "deadline"),
            note("b2", Some(40_000), "rollback"),
        ];
        let utterances = vec![
            utterance(1, 8_000, "the deadline is Thursday"),
            utterance(2, 38_000, "I'll own the rollback plan"),
        ];

        let links = link_layered(&notes, &utterances, &params);
        let b1 = links.iter().find(|l| l.note_block_id == "b1").unwrap();
        let b2 = links.iter().find(|l| l.note_block_id == "b2").unwrap();
        assert_eq!(b1.utterance_id, 1);
        assert_eq!(b2.utterance_id, 2);
    }

    #[test]
    fn a_meeting_with_no_transcript_produces_no_links() {
        let params = LinkParams::default();
        assert!(link_layered(&[note("b1", Some(1_000), "x")], &[], &params).is_empty());
        assert!(link_baseline(&[note("b1", Some(1_000), "x")], &[], &params).is_empty());
    }

    // ------------------------------------------------------------------ llm

    #[test]
    fn an_llm_citation_that_agrees_promotes_the_existing_link() {
        // Agreement between layers is the strongest signal available; a second
        // row for the same pair would just render twice.
        let mut links = vec![Link {
            note_block_id: "b1".into(),
            utterance_id: 12,
            method: LinkMethod::Temporal,
            score: 0.5,
        }];
        merge_llm_links(&mut links, &[("b1".into(), 12)]);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].method, LinkMethod::Llm);
        assert!(links[0].score >= 1.0);
    }

    #[test]
    fn an_llm_citation_the_other_layers_missed_is_added() {
        let mut links = vec![];
        merge_llm_links(&mut links, &[("b1".into(), 99)]);
        assert_eq!(links[0].utterance_id, 99);
        assert_eq!(links[0].method, LinkMethod::Llm);
    }

    #[test]
    fn the_method_is_recorded_so_quality_can_be_attributed() {
        // Without this, "is the semantic layer helping?" is unanswerable.
        let params = LinkParams::default();
        let links = link_layered(
            &[note("b1", None, "deadline migration Thursday")],
            &[utterance(
                1,
                1_000,
                "the deadline for the migration is Thursday",
            )],
            &params,
        );
        assert_eq!(links[0].method, LinkMethod::Semantic);
    }
}
