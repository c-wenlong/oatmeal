//! Runs a search against a real database and prints what came back.
//!
//! The instrument for the question G24 exists to answer: does a phrase you
//! half-remember find the right meeting and the right moment? Unit tests use a
//! bag-of-words stand-in embedder, which cannot tell you that — only a real
//! index over real transcripts can.
//!
//! ```text
//! cargo run --example trysearch -- "$HOME/Library/Application Support/com.kaichen.oatmeal/oatmeal.sqlite" "shrink the scope"
//! ```

use oatmeal_lib::db::Database;
use oatmeal_lib::embed::HttpEmbedder;
use oatmeal_lib::search;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: trysearch <db path> <query>");
    let query: String = args.collect::<Vec<_>>().join(" ");
    assert!(!query.trim().is_empty(), "need a query");

    let db = Database::open(&path).expect("could not open the database");
    let response = search::search(db.connection(), &query, None, &HttpEmbedder::local(), 10)
        .await
        .expect("search failed");

    println!(
        "query: {query:?}   searched by meaning: {}",
        response.semantic
    );
    if response.results.is_empty() {
        println!("(nothing matched)");
    }

    for result in &response.results {
        println!(
            "\n## {}  ({} hit(s), best at {}ms)",
            result
                .meeting
                .title
                .clone()
                .unwrap_or_else(|| "Untitled".into()),
            result.meeting.hits.len(),
            result.meeting.best_at_ms
        );
        for (hit, preview) in result.meeting.hits.iter().zip(&result.previews) {
            let marked: Vec<String> = preview
                .spans
                .iter()
                .map(|(start, end)| {
                    preview
                        .text
                        .chars()
                        .skip(*start)
                        .take(end - start)
                        .collect()
                })
                .collect();
            println!("   [{:?}] {}", hit.kind, preview.text);
            if !marked.is_empty() {
                println!("           highlighted: {marked:?}");
            }
        }
    }
}
