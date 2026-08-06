//! The semantic layer, measured against real meetings.
//!
//! `eval` scores the *whole* linker on a fixture: fourteen cases the author
//! wrote, matched by a bag-of-words stand-in embedder. It answers "do the
//! layers combine correctly", and it is honest about being a harness rather
//! than evidence.
//!
//! This module answers a different question that the fixture cannot: **does
//! the semantic layer work on real language, with the real embedder?**
//!
//! **Why the metric is meeting attribution.** Scoring "did the note link to
//! the right *line*" needs a human to say which line is right, and no such
//! labels exist. But every note already carries one label for free — the
//! meeting it was taken in. So: embed every line from every meeting into one
//! pool, embed a note, and ask whether its nearest neighbour came from its own
//! meeting. Nothing is hand-labelled and nothing can be tuned to fit.
//!
//! **What it does not measure.** Picking the right line *within* a meeting is
//! the linker's actual job, and attribution is a proxy for it — a coarse one.
//! A note that retrieves the correct meeting may still land on the wrong line
//! in it. Treat a good score here as "the embedder discriminates real meeting
//! content", not as "links are correct".
//!
//! **The corpus is not in the repo.** It is real meeting data. The path
//! arrives through `OATMEAL_BENCH_CORPUS` and nothing personal is committed.

use serde::Deserialize;

use crate::embed::{cosine, EmbedError, Embedder};

/// One meeting: what was said, and what was written down about it.
#[derive(Debug, Clone, Deserialize)]
pub struct Meeting {
    #[serde(default)]
    pub title: String,
    pub notes: Vec<String>,
    pub utterances: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Corpus {
    pub meetings: Vec<Meeting>,
}

impl Corpus {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Loads the corpus named by `OATMEAL_BENCH_CORPUS`.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var("OATMEAL_BENCH_CORPUS").ok()?;
        let json = std::fs::read_to_string(path).ok()?;
        Self::from_json(&json).ok()
    }

    pub fn note_count(&self) -> usize {
        self.meetings.iter().map(|m| m.notes.len()).sum()
    }

    pub fn utterance_count(&self) -> usize {
        self.meetings.iter().map(|m| m.utterances.len()).sum()
    }
}

/// How often a note's nearest lines came from the meeting it was taken in.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Attribution {
    /// The single nearest line was from the note's own meeting.
    pub top1: usize,
    /// The own meeting appeared anywhere in the five nearest lines.
    pub top5: usize,
    pub total: usize,
}

impl Attribution {
    pub fn rate1(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        self.top1 as f32 / self.total as f32
    }

    pub fn rate5(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        self.top5 as f32 / self.total as f32
    }
}

/// What a coin flip would score.
///
/// Not `1/meetings` — meetings differ in length, so drawing a line at random
/// favours the long ones. The honest baseline is the chance that a uniformly
/// random *line* belongs to the note's own meeting, averaged over notes. A
/// benchmark whose baseline is wrong flatters or maligns the thing it measures.
pub fn chance(owners: &[usize], note_owners: &[usize]) -> f32 {
    if owners.is_empty() || note_owners.is_empty() {
        return 0.0;
    }
    let total = owners.len() as f32;
    let sum: f32 = note_owners
        .iter()
        .map(|owner| owners.iter().filter(|o| *o == owner).count() as f32 / total)
        .sum();
    sum / note_owners.len() as f32
}

/// Scores one set of note vectors against one pool of line vectors.
///
/// Kept free of embedding and I/O so the scoring rule itself is testable with
/// hand-written vectors.
pub fn attribute(notes: &[(usize, Vec<f32>)], lines: &[(usize, Vec<f32>)]) -> Attribution {
    let mut result = Attribution::default();
    for (owner, note) in notes {
        if lines.is_empty() {
            continue;
        }
        result.total += 1;

        // Rank every line by similarity, then look at the head of the list.
        let mut scored: Vec<(usize, f32)> = lines
            .iter()
            .map(|(line_owner, vector)| (*line_owner, cosine(note, vector)))
            .collect();
        // Descending, and ties broken deterministically so a rerun on the same
        // vectors cannot report a different number.
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

        if scored[0].0 == *owner {
            result.top1 += 1;
        }
        if scored.iter().take(5).any(|(o, _)| o == owner) {
            result.top5 += 1;
        }
    }
    result
}

/// Embeds in batches.
///
/// One request carrying five thousand texts is refused or times out; the
/// pipeline settled on 64 for the same reason.
const BATCH: usize = 64;

pub async fn embed_all<E: Embedder>(
    embedder: &E,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, EmbedError> {
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(BATCH) {
        out.extend(embedder.embed(chunk).await?);
    }
    Ok(out)
}

/// Texts paired with the index of the meeting they came from.
///
/// That index is the entire ground truth of this benchmark, so it travels with
/// the text everywhere rather than being tracked alongside it — a reordering
/// that silently separated the two would produce a plausible score that means
/// nothing.
pub type Owned = Vec<(usize, String)>;

/// The same pairing after embedding.
pub type Embedded = Vec<(usize, Vec<f32>)>;

/// Flattens the corpus into (owning meeting, text) pairs.
pub fn flatten(corpus: &Corpus) -> (Owned, Owned) {
    let mut notes = Vec::new();
    let mut lines = Vec::new();
    for (index, meeting) in corpus.meetings.iter().enumerate() {
        notes.extend(meeting.notes.iter().map(|n| (index, n.clone())));
        lines.extend(meeting.utterances.iter().map(|u| (index, u.clone())));
    }
    (notes, lines)
}

/// Embeds both sides and scores them.
pub async fn run<E: Embedder>(embedder: &E, corpus: &Corpus) -> Result<Attribution, EmbedError> {
    let (notes, lines) = flatten(corpus);
    let note_vectors = embed_all(
        embedder,
        &notes.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>(),
    )
    .await?;
    let line_vectors = embed_all(
        embedder,
        &lines.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>(),
    )
    .await?;

    let notes: Embedded = notes.iter().map(|(o, _)| *o).zip(note_vectors).collect();
    let lines: Embedded = lines.iter().map(|(o, _)| *o).zip(line_vectors).collect();

    Ok(attribute(&notes, &lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(values: &[f32]) -> Vec<f32> {
        values.to_vec()
    }

    #[test]
    fn a_note_whose_nearest_line_is_its_own_meeting_counts() {
        let notes = vec![(0usize, v(&[1.0, 0.0]))];
        let lines = vec![(0usize, v(&[1.0, 0.0])), (1usize, v(&[0.0, 1.0]))];
        let scored = attribute(&notes, &lines);
        assert_eq!(scored.top1, 1);
        assert_eq!(scored.total, 1);
    }

    #[test]
    fn a_note_that_lands_in_the_wrong_meeting_does_not_count() {
        // The whole benchmark is worthless if a miss can score.
        let notes = vec![(0usize, v(&[0.0, 1.0]))];
        let lines = vec![(0usize, v(&[1.0, 0.0])), (1usize, v(&[0.0, 1.0]))];
        let scored = attribute(&notes, &lines);
        assert_eq!(scored.top1, 0);
        assert_eq!(scored.total, 1);
    }

    #[test]
    fn top5_is_lenient_where_top1_is_strict() {
        // The right meeting sits fourth. top5 should forgive that; top1 must not.
        let notes = vec![(9usize, v(&[1.0, 0.0]))];
        let lines = vec![
            (0, v(&[0.99, 0.14])),
            (1, v(&[0.98, 0.19])),
            (2, v(&[0.97, 0.24])),
            (9, v(&[0.96, 0.28])),
            (3, v(&[0.0, 1.0])),
        ];
        let scored = attribute(&notes, &lines);
        assert_eq!(scored.top1, 0, "the nearest line was not the own meeting");
        assert_eq!(scored.top5, 1, "but it was inside the top five");
    }

    #[test]
    fn a_note_beyond_the_fifth_line_misses_both() {
        let notes = vec![(9usize, v(&[1.0, 0.0]))];
        let mut lines: Vec<(usize, Vec<f32>)> = (0..6)
            .map(|i| (i, v(&[1.0 - i as f32 * 0.01, i as f32 * 0.05])))
            .collect();
        lines.push((9, v(&[0.0, 1.0])));
        let scored = attribute(&notes, &lines);
        assert_eq!((scored.top1, scored.top5), (0, 0));
    }

    #[test]
    fn scoring_is_stable_when_similarities_tie() {
        // Two lines from different meetings, identical vectors. Without a tie
        // break the winner depends on sort order and the benchmark reports a
        // different number on a rerun of the same data.
        let notes = vec![(1usize, v(&[1.0, 0.0]))];
        let lines = vec![(0usize, v(&[1.0, 0.0])), (1usize, v(&[1.0, 0.0]))];
        let first = attribute(&notes, &lines);
        let second = attribute(&notes, &lines);
        assert_eq!(first, second);
    }

    #[test]
    fn an_empty_pool_scores_nothing_rather_than_panicking() {
        let scored = attribute(&[(0usize, v(&[1.0, 0.0]))], &[]);
        assert_eq!(scored, Attribution::default());
        assert_eq!(scored.rate1(), 0.0);
    }

    #[test]
    fn chance_follows_meeting_size_not_meeting_count() {
        // Meeting 0 owns three of four lines. A note from it beats a note from
        // meeting 1 by luck alone, and `1/meetings` (0.5) would hide that.
        let owners = vec![0, 0, 0, 1];
        assert!((chance(&owners, &[0]) - 0.75).abs() < 1e-6);
        assert!((chance(&owners, &[1]) - 0.25).abs() < 1e-6);
        // Averaged over one note from each.
        assert!((chance(&owners, &[0, 1]) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn chance_of_an_empty_corpus_is_zero_not_a_divide_by_zero() {
        assert_eq!(chance(&[], &[0]), 0.0);
        assert_eq!(chance(&[0], &[]), 0.0);
    }

    #[test]
    fn rates_are_fractions_of_the_notes_scored() {
        let scored = Attribution {
            top1: 3,
            top5: 4,
            total: 8,
        };
        assert!((scored.rate1() - 0.375).abs() < 1e-6);
        assert!((scored.rate5() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_corpus_parses_and_flattens_with_its_owners_intact() {
        let corpus = Corpus::from_json(
            r#"{"meetings":[
                 {"title":"a","notes":["n1","n2"],"utterances":["u1"]},
                 {"title":"b","notes":["n3"],"utterances":["u2","u3"]}]}"#,
        )
        .unwrap();
        assert_eq!((corpus.note_count(), corpus.utterance_count()), (3, 3));

        let (notes, lines) = flatten(&corpus);
        // Losing the owner index would silently make every attribution wrong.
        assert_eq!(notes.iter().map(|(o, _)| *o).collect::<Vec<_>>(), [0, 0, 1]);
        assert_eq!(lines.iter().map(|(o, _)| *o).collect::<Vec<_>>(), [0, 1, 1]);
    }

    #[tokio::test]
    async fn embedding_happens_in_batches() {
        // A single request carrying every line is refused by the server. This
        // pins the batching rather than trusting it.
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(AtomicUsize);
        impl Embedder for Counting {
            async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
                assert!(
                    texts.len() <= BATCH,
                    "batch of {} exceeds {BATCH}",
                    texts.len()
                );
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
            }
        }

        let embedder = Counting(AtomicUsize::new(0));
        let texts: Vec<String> = (0..(BATCH * 2 + 1)).map(|i| i.to_string()).collect();
        let out = embed_all(&embedder, &texts).await.unwrap();
        assert_eq!(out.len(), texts.len());
        assert_eq!(embedder.0.load(Ordering::SeqCst), 3);
    }

    /// The real measurement. Needs Ollama and the corpus:
    ///
    /// ```text
    /// OATMEAL_BENCH_CORPUS=/path/corpus.json \
    ///   cargo test --lib bench::tests::live -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore]
    async fn live_the_real_embedder_is_measured_against_the_stand_in() {
        use crate::embed::{HashEmbedder, HttpEmbedder};

        let Some(corpus) = Corpus::from_env() else {
            panic!("set OATMEAL_BENCH_CORPUS to a corpus JSON");
        };
        let (notes, lines) = flatten(&corpus);
        let note_owners: Vec<usize> = notes.iter().map(|(o, _)| *o).collect();
        let line_owners: Vec<usize> = lines.iter().map(|(o, _)| *o).collect();

        eprintln!(
            "corpus: {} meetings, {} notes, {} lines",
            corpus.meetings.len(),
            corpus.note_count(),
            corpus.utterance_count()
        );
        eprintln!("chance: {:.1}%", chance(&line_owners, &note_owners) * 100.0);

        let started = std::time::Instant::now();
        let stand_in = run(&HashEmbedder, &corpus).await.expect("hash embedder");
        eprintln!(
            "bag-of-words  top1 {:>5.1}%  top5 {:>5.1}%   ({:.1}s)",
            stand_in.rate1() * 100.0,
            stand_in.rate5() * 100.0,
            started.elapsed().as_secs_f32()
        );

        let started = std::time::Instant::now();
        let real = run(&HttpEmbedder::local(), &corpus)
            .await
            .expect("is Ollama running with nomic-embed-text:v1.5?");
        eprintln!(
            "nomic-embed   top1 {:>5.1}%  top5 {:>5.1}%   ({:.1}s)",
            real.rate1() * 100.0,
            real.rate5() * 100.0,
            started.elapsed().as_secs_f32()
        );

        // The claim under test: the real embedder beats chance by a wide
        // margin on real meetings. Anything less and the semantic layer is not
        // earning its place.
        assert!(
            real.rate1() > chance(&line_owners, &note_owners) * 3.0,
            "the real embedder barely beat chance"
        );
    }
}
