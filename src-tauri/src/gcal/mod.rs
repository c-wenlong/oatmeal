//! Google Calendar over OAuth, for people whose calendar is not in macOS.
//!
//! EventKit (G20) remains the default and covers most users: if the account is
//! in macOS Calendar, nothing here is needed. This exists for the case EventKit
//! cannot reach, and is strictly additive — turning it off leaves detection
//! exactly as it was.
//!
//! **No client secret is shipped.** Google documents the secret as
//! inapplicable to installed apps, and PKCE replaces what it was doing. What
//! ships is a client *id*, which the user creates themselves.

pub mod connection;
pub mod events;
pub mod loopback;
pub mod pkce;
pub mod token;

pub use connection::Connection;
pub use pkce::Pkce;
pub use token::{TokenError, Tokens};
