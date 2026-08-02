//! Runs the linking backfill against a real database.
//!
//! The app indexes automatically when a meeting ends, which is no help for
//! meetings recorded before Phase 4 existed. This is the manual trigger, and
//! also the way to exercise the real embedding model against real data without
//! going through the UI:
//!
//! ```text
//! cargo run --example backfill -- "$HOME/Library/Application Support/com.kaichen.oatmeal/oatmeal.sqlite"
//! ```

use oatmeal_lib::db::Database;
use oatmeal_lib::embed::HttpEmbedder;
use oatmeal_lib::link::{pipeline, LinkParams};

#[tokio::main]
async fn main() {
    let path = std::env::args().nth(1).expect("usage: backfill <db path>");
    let mut db = Database::open(&path).expect("could not open database");

    let started = std::time::Instant::now();
    let reports = pipeline::backfill(
        db.connection_mut(),
        &HttpEmbedder::local(),
        &LinkParams::default(),
        100,
    )
    .await
    .expect("backfill failed");

    for (meeting_id, report) in &reports {
        println!(
            "{meeting_id}: embedded {}, {} links{}",
            report.embedded,
            report.links,
            report
                .degraded
                .as_deref()
                .map(|r| format!(" (timestamps only — {r})"))
                .unwrap_or_default()
        );
    }
    println!(
        "{} meetings in {:.1}s",
        reports.len(),
        started.elapsed().as_secs_f32()
    );
}
