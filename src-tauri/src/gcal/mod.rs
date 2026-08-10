//! Google Calendar over OAuth, for people whose calendar is not in macOS.
//!
//! EventKit (G20) remains the default and covers most users: if the account is
//! in macOS Calendar, nothing here is needed. This exists for the case EventKit
//! cannot reach, and is strictly additive — turning it off leaves detection
//! exactly as it was.
//!
//! **Nothing confidential ships in the binary.** Both halves of the credential
//! are the user's own, from their own Google Cloud project.
//!
//! The client *secret* is required all the same. Google documents it as
//! non-confidential for installed apps, which this module first read as
//! optional — it is not, and the token endpoint answers a request without it
//! with `invalid_request: client_secret is missing.` PKCE is still what carries
//! the security: a secret that ships in a desktop binary can be read out of it
//! with `strings`, and the verifier cannot.

pub mod connection;
pub mod events;
pub mod loopback;
pub mod pkce;
pub mod token;

pub use connection::Connection;
pub use pkce::Pkce;
pub use token::{TokenError, Tokens};
