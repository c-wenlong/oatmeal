//! Exporting a meeting to Notion.
//!
//! The user named Notion as one of two minimum connectors, and the shape is the
//! documented default: **one page per meeting in a database they choose**.
//!
//! The property is that re-exporting *updates*. A user who regenerates a summary
//! and exports again should see the same page change, not acquire a second one —
//! so the page id is remembered per meeting, and every export after the first is
//! an update to that page.

pub mod client;
pub mod export;
pub mod shape;

pub use client::{Notion, NotionError};
