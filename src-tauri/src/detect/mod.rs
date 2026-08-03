//! Meeting detection: what counts as a meeting starting, and what to do about it.
//!
//! Three trigger sources feed one queue ([`queue`]), and a per-app policy
//! ([`rules`]) decides whether a microphone activation is allowed to say
//! anything at all. Both halves are pure — no Tauri, no database, no clock — so
//! the parts that are easy to get subtly wrong are the parts that are tested.
//!
//! The invariant that matters most: **detection never records.** Everything
//! here produces an offer, and an offer needs a click.

pub mod calendar;
pub mod queue;
pub mod rules;

pub use calendar::CalendarEvent;
pub use queue::{Candidate, Outcome, Queue, Source};
pub use rules::{Decision, RuleMode};
