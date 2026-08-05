//! PKCE, and the parts of the OAuth dance that are pure computation.
//!
//! **Why PKCE at all.** A desktop app cannot keep a secret: anything shipped in
//! the binary can be read out of it with `strings`. PKCE (RFC 7636) replaces the
//! client secret with a value invented per-attempt — the app proves it started
//! the flow by producing the pre-image of a hash it sent earlier, and an
//! attacker who intercepts the authorization code cannot redeem it without that
//! pre-image.
//!
//! So Oatmeal ships a client *id*, which Google documents as non-confidential
//! for installed apps, and no secret at all.

use base64::Engine;
use sha2::{Digest, Sha256};

/// The scope this asks for.
///
/// Read-only, and only events. `calendar.readonly` would also expose calendar
/// settings and ACLs, which detection has no use for — and a consent screen
/// asking for more than the feature needs is how a user learns not to trust the
/// app.
pub const SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.readonly";

pub const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// A verifier and its challenge, generated together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    /// Sent only in the token exchange, never in the browser.
    pub verifier: String,
    /// Sent in the authorization URL, which is visible to the browser and to
    /// anything watching it. Safe, because it is a one-way hash.
    pub challenge: String,
}

/// Base64url without padding — what RFC 7636 specifies.
fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

impl Pkce {
    /// A fresh verifier and its S256 challenge.
    ///
    /// 32 random bytes become 43 base64url characters — the shortest length RFC
    /// 7636 allows, and 256 bits of entropy, which is plenty. A verifier that
    /// repeats between attempts would defeat the whole mechanism, so this must
    /// never be cached or derived from anything stable.
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self::from_bytes(&bytes)
    }

    /// Deterministic construction, so the transform can be tested against the
    /// worked example in RFC 7636.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let verifier = b64url(bytes);
        Self::from_verifier(verifier)
    }

    pub fn from_verifier(verifier: String) -> Self {
        let digest = Sha256::digest(verifier.as_bytes());
        Self {
            verifier,
            challenge: b64url(&digest),
        }
    }
}

/// Opaque value tying the callback to the request that started it.
///
/// Separate from the PKCE verifier and serving a different purpose: PKCE proves
/// *we* started the flow, `state` proves *this callback belongs to it*. Without
/// it, anything that can reach the loopback server could hand us an
/// authorization code from a different account and we would redeem it.
pub fn random_state() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    b64url(&bytes)
}

/// Percent-encodes a query parameter value.
///
/// Hand-rolled because the alternative is a dependency for one function, and
/// the rule is short: everything that is not unreserved gets escaped. Scopes
/// contain `:` and `/`, and a redirect URI contains both — encoding them wrong
/// produces a `redirect_uri_mismatch` that reads like a configuration error.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The URL to open in the user's browser.
pub fn authorize_url(client_id: &str, redirect_uri: &str, pkce: &Pkce, state: &str) -> String {
    format!(
        "{AUTH_ENDPOINT}?response_type=code\
         &client_id={}\
         &redirect_uri={}\
         &scope={}\
         &code_challenge={}\
         &code_challenge_method=S256\
         &state={}\
         &access_type=offline\
         &prompt=consent",
        encode(client_id),
        encode(redirect_uri),
        encode(SCOPE),
        encode(&pkce.challenge),
        encode(state),
    )
}

/// What came back on the loopback redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    Code {
        code: String,
        state: String,
    },
    /// The user pressed Cancel, or Google refused.
    Denied {
        error: String,
        state: Option<String>,
    },
}

/// Parses the query string of the redirect.
pub fn parse_callback(query: &str) -> Option<Callback> {
    let mut code = None;
    let mut state = None;
    let mut error = None;

    for pair in query.trim_start_matches('?').split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = percent_decode(value);
        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }

    if let Some(error) = error {
        return Some(Callback::Denied { error, state });
    }
    match (code, state) {
        (Some(code), Some(state)) => Some(Callback::Code { code, state }),
        // A code with no state cannot be tied to the request that started it,
        // so it is refused rather than redeemed.
        _ => None,
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                16,
            ) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether a callback belongs to the flow we started.
///
/// Constant-time-ish is not required here — `state` is not a secret, it is a
/// correlator — but the check itself is mandatory.
pub fn state_matches(expected: &str, actual: &str) -> bool {
    !expected.is_empty() && expected == actual
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_matches_the_rfc_worked_example() {
        // RFC 7636 appendix B. If this drifts, every authorization is rejected
        // by Google with an error that says nothing about which half is wrong.
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_string());
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_generated_verifier_is_the_length_the_rfc_allows() {
        // 43 to 128 characters. 32 random bytes base64url to exactly 43.
        let pkce = Pkce::generate();
        assert_eq!(pkce.verifier.len(), 43);
        assert!(pkce.verifier.len() >= 43 && pkce.verifier.len() <= 128);
    }

    #[test]
    fn the_verifier_is_never_padded() {
        // A `=` would be re-encoded in the token request and no longer match
        // the challenge Google stored.
        let pkce = Pkce::from_bytes(&[0u8; 32]);
        assert!(!pkce.verifier.contains('='));
        assert!(!pkce.challenge.contains('='));
    }

    #[test]
    fn the_verifier_is_url_safe() {
        // Standard base64 would emit `+` and `/`, which change meaning in a
        // query string.
        let pkce = Pkce::from_bytes(&[251, 255, 190, 0, 1, 2, 3, 4]);
        assert!(!pkce.verifier.contains('+') && !pkce.verifier.contains('/'));
        assert!(!pkce.challenge.contains('+') && !pkce.challenge.contains('/'));
    }

    #[test]
    fn two_flows_never_share_a_verifier() {
        // Reusing one would defeat the entire mechanism.
        assert_ne!(Pkce::generate().verifier, Pkce::generate().verifier);
        assert_ne!(random_state(), random_state());
    }

    #[test]
    fn the_scope_is_the_narrowest_one_that_works() {
        // Detection reads events. Asking for `calendar.readonly` would also
        // expose settings and ACLs, and a consent screen that asks for more
        // than the feature needs teaches people not to trust the app.
        assert_eq!(
            SCOPE,
            "https://www.googleapis.com/auth/calendar.events.readonly"
        );
        assert!(SCOPE.ends_with("readonly"), "the scope must be read-only");
    }

    #[test]
    fn the_authorize_url_carries_everything_google_needs() {
        let pkce = Pkce::from_verifier("v".repeat(43));
        let url = authorize_url("client-123", "http://127.0.0.1:9999", &pkce, "st4te");

        assert!(url.starts_with(AUTH_ENDPOINT));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={}", encode(&pkce.challenge))));
        assert!(url.contains("state=st4te"));
    }

    #[test]
    fn the_authorize_url_asks_for_a_refresh_token() {
        // Without `access_type=offline` Google returns only an access token,
        // and the connection silently dies an hour later.
        let url = authorize_url("c", "http://127.0.0.1:1", &Pkce::generate(), "s");
        assert!(url.contains("access_type=offline"));
        // And `prompt=consent`, or a second authorization returns no refresh
        // token at all because one was already issued.
        assert!(url.contains("prompt=consent"));
    }

    #[test]
    fn the_verifier_never_appears_in_the_browser_url() {
        // The whole point: the browser sees the hash, not the pre-image.
        let pkce = Pkce::generate();
        let url = authorize_url("c", "http://127.0.0.1:1", &pkce, "s");
        assert!(
            !url.contains(&pkce.verifier),
            "the PKCE verifier leaked into the authorization URL"
        );
    }

    #[test]
    fn the_redirect_uri_is_encoded() {
        // Unencoded `:` and `/` produce a redirect_uri_mismatch that reads like
        // a console misconfiguration rather than an encoding bug.
        let url = authorize_url("c", "http://127.0.0.1:9999", &Pkce::generate(), "s");
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A9999"));
    }

    #[test]
    fn a_successful_callback_parses() {
        let callback = parse_callback("?code=4%2F0Abc_def&state=xyz").unwrap();
        assert_eq!(
            callback,
            Callback::Code {
                code: "4/0Abc_def".into(),
                state: "xyz".into()
            }
        );
    }

    #[test]
    fn a_denial_parses_rather_than_hanging() {
        // Pressing Cancel must close the flow, not leave the app waiting.
        let callback = parse_callback("?error=access_denied&state=xyz").unwrap();
        assert_eq!(
            callback,
            Callback::Denied {
                error: "access_denied".into(),
                state: Some("xyz".into())
            }
        );
    }

    #[test]
    fn a_code_with_no_state_is_refused() {
        // It cannot be tied to the flow we started, so it must not be redeemed.
        assert_eq!(parse_callback("?code=abc"), None);
    }

    #[test]
    fn junk_is_refused() {
        assert_eq!(parse_callback(""), None);
        assert_eq!(parse_callback("?"), None);
        assert_eq!(parse_callback("?nonsense"), None);
    }

    #[test]
    fn a_mismatched_state_is_rejected() {
        // The attack this prevents: someone reaches the loopback server and
        // hands us an authorization code belonging to a different account.
        assert!(state_matches("abc", "abc"));
        assert!(!state_matches("abc", "abd"));
        assert!(!state_matches("", ""), "an empty state must never match");
    }
}
