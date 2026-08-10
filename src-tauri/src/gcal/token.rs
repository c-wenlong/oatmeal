//! Exchanging the code, and keeping the connection alive afterwards.
//!
//! Two tokens with very different lifetimes and very different handling:
//!
//! - The **refresh token** is long-lived and is the actual credential. It goes
//!   in the Keychain, next to the LLM keys, and never touches SQLite.
//! - The **access token** lasts about an hour. It is held in memory only —
//!   persisting it would put a bearer token on disk to save one round trip.

use serde::{Deserialize, Serialize};

use super::pkce::TOKEN_ENDPOINT;

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("could not reach Google: {0}")]
    Unreachable(String),
    #[error("Google rejected the request: {0}")]
    Rejected(String),
    #[error("unexpected response from Google: {0}")]
    Malformed(String),
    #[error("not connected to Google Calendar")]
    NotConnected,
    /// The refresh token no longer works. Distinct from every other failure
    /// because it is the only one the user has to act on.
    #[error("the connection was revoked or expired — reconnect to continue")]
    NeedsReauth,
}

/// What Google returns from the token endpoint.
#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// A live connection.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tokens {
    pub access_token: String,
    /// Absent on a refresh — Google only issues one at first authorization, so
    /// the stored one must be kept rather than overwritten with nothing.
    pub refresh_token: Option<String>,
    /// Wall clock at which the access token stops working.
    pub expires_at_ms: i64,
}

/// How early to treat an access token as expired.
///
/// A token that expires during a request in flight fails the request. Sixty
/// seconds of slack costs one extra refresh an hour and removes the whole class
/// of "it worked a second ago" failures.
pub const EXPIRY_SLACK_MS: i64 = 60_000;

impl Tokens {
    pub fn is_expired(&self, now_ms: i64) -> bool {
        now_ms + EXPIRY_SLACK_MS >= self.expires_at_ms
    }
}

fn to_tokens(response: TokenResponse, now_ms: i64) -> Tokens {
    Tokens {
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        // Google always sends `expires_in`, but a missing one must not mean
        // "never expires" — that would wedge the connection permanently.
        expires_at_ms: now_ms + response.expires_in.unwrap_or(3600) * 1000,
    }
}

async fn post(
    http: &reqwest::Client,
    params: &[(&str, &str)],
) -> Result<TokenResponse, TokenError> {
    let response = http
        .post(TOKEN_ENDPOINT)
        .form(params)
        .send()
        .await
        .map_err(|e| TokenError::Unreachable(e.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| TokenError::Unreachable(e.to_string()))?;

    if !status.is_success() {
        let error = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| body.chars().take(200).collect());

        // `invalid_grant` is Google's answer for a refresh token that has been
        // revoked, expired, or invalidated by a password change. It is the one
        // failure that needs the user, so it is not lumped in with the rest.
        if error == "invalid_grant" {
            return Err(TokenError::NeedsReauth);
        }
        return Err(TokenError::Rejected(error));
    }

    serde_json::from_str(&body).map_err(|e| TokenError::Malformed(e.to_string()))
}

/// The form body for redeeming a code.
///
/// Split out from the request so the one thing that is easy to get wrong can be
/// asserted without a network: an **absent** `client_secret` and an **empty**
/// one are different requests, and Google answers the empty one with the same
/// `invalid_request` it gives for a missing one.
fn exchange_params<'a>(
    client_id: &'a str,
    client_secret: Option<&'a str>,
    code: &'a str,
    verifier: &'a str,
    redirect_uri: &'a str,
) -> Vec<(&'a str, &'a str)> {
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(secret) = client_secret.filter(|s| !s.trim().is_empty()) {
        params.push(("client_secret", secret));
    }
    params
}

/// Redeems the authorization code.
///
/// **`client_secret` is required for Google's Desktop app clients**, PKCE or
/// not. Google documents the secret as *not confidential* for installed apps —
/// which is true, and which this code originally misread as *not required*.
/// The token endpoint answers a request without it:
///
/// ```text
/// {"error": "invalid_request", "error_description": "client_secret is missing."}
/// ```
///
/// So it is sent, and stored in the Keychain rather than SQLite: Google calling
/// it non-confidential is not a reason to leave a credential in a plain file.
/// PKCE stays, and is still doing its job — the secret is extractable from any
/// desktop binary, and the verifier is what an interceptor cannot produce.
///
/// Optional because it genuinely is for some client types: Google issues no
/// secret at all for iOS and Android clients.
pub async fn exchange(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    now_ms: i64,
) -> Result<Tokens, TokenError> {
    let response = post(
        http,
        &exchange_params(client_id, client_secret, code, verifier, redirect_uri),
    )
    .await?;

    if response.refresh_token.is_none() {
        // Without one the connection dies in an hour with no way back except a
        // fresh authorization, which is worse than failing now and saying so.
        return Err(TokenError::Malformed(
            "Google returned no refresh token — the app may already be authorized; \
             remove it at myaccount.google.com/permissions and try again"
                .into(),
        ));
    }
    Ok(to_tokens(response, now_ms))
}

/// The form body for a refresh. Same absent-versus-empty care as the exchange.
fn refresh_params<'a>(
    client_id: &'a str,
    client_secret: Option<&'a str>,
    refresh_token: &'a str,
) -> Vec<(&'a str, &'a str)> {
    let mut params = vec![
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_token),
    ];
    if let Some(secret) = client_secret.filter(|s| !s.trim().is_empty()) {
        params.push(("client_secret", secret));
    }
    params
}

/// Trades the refresh token for a new access token.
///
/// Takes the secret for the same reason `exchange` does. Omitting it here would
/// be the worse bug of the two: the connection would appear to work and then
/// break an hour later, when nobody is watching.
pub async fn refresh(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
    now_ms: i64,
) -> Result<Tokens, TokenError> {
    let response = post(
        http,
        &refresh_params(client_id, client_secret, refresh_token),
    )
    .await?;

    let mut tokens = to_tokens(response, now_ms);
    // A refresh response carries no refresh token. Keeping the one we already
    // have is the difference between a connection that lasts and one that dies
    // on the first refresh.
    tokens.refresh_token = Some(refresh_token.to_string());
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(expires_at_ms: i64) -> Tokens {
        Tokens {
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            expires_at_ms,
        }
    }

    #[test]
    fn a_token_is_expired_before_it_actually_expires() {
        // A token that dies mid-request fails the request; a minute of slack
        // removes the whole class of "it worked a second ago".
        let token = tokens(100_000);
        assert!(!token.is_expired(100_000 - EXPIRY_SLACK_MS - 1));
        assert!(token.is_expired(100_000 - EXPIRY_SLACK_MS));
        assert!(token.is_expired(100_000));
    }

    #[test]
    fn a_response_without_expiry_still_expires() {
        // Treating a missing `expires_in` as "never" would wedge the connection
        // permanently on a token that stopped working an hour ago.
        let converted = to_tokens(
            TokenResponse {
                access_token: "at".into(),
                refresh_token: Some("rt".into()),
                expires_in: None,
            },
            0,
        );
        assert_eq!(converted.expires_at_ms, 3_600_000);
    }

    #[test]
    fn expiry_is_computed_from_now() {
        let converted = to_tokens(
            TokenResponse {
                access_token: "at".into(),
                refresh_token: None,
                expires_in: Some(3599),
            },
            1_000_000,
        );
        assert_eq!(converted.expires_at_ms, 1_000_000 + 3_599_000);
    }

    #[tokio::test]
    async fn an_unreachable_google_says_so() {
        // Points at a closed port rather than the real endpoint.
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(300))
            .build()
            .unwrap();
        let err = post(&http, &[("grant_type", "refresh_token")]).await.err();
        // Either unreachable or rejected depending on the network; what must
        // not happen is a panic or a hang.
        assert!(err.is_some());
    }

    /// What a param list carries for one key.
    fn value<'a>(params: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
        params.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    #[test]
    fn the_exchange_carries_the_client_secret() {
        // Google's Desktop app clients require it, PKCE or not. Leaving it out
        // is what produced `invalid_request: client_secret is missing.` and a
        // Connect button that could never succeed.
        let params = exchange_params("id", Some("shh"), "code", "verifier", "http://127.0.0.1:1");
        assert_eq!(value(&params, "client_secret"), Some("shh"));
        // And PKCE is still there — the secret does not replace it.
        assert_eq!(value(&params, "code_verifier"), Some("verifier"));
    }

    #[test]
    fn a_refresh_carries_it_too() {
        // Sending it only on the exchange would connect fine and then break an
        // hour later, on the first refresh, with nobody watching.
        let params = refresh_params("id", Some("shh"), "rt");
        assert_eq!(value(&params, "client_secret"), Some("shh"));
    }

    #[test]
    fn an_empty_secret_is_left_out_rather_than_sent_empty() {
        // The same empty-versus-absent trap that broke signing and notarization
        // in CI twice. `client_secret=` is not "no secret" to Google; it is a
        // secret that is wrong.
        for empty in ["", "   "] {
            let params = exchange_params("id", Some(empty), "c", "v", "r");
            assert_eq!(value(&params, "client_secret"), None);
            assert_eq!(
                value(&refresh_params("id", Some(empty), "rt"), "client_secret"),
                None
            );
        }
    }

    #[test]
    fn a_client_with_no_secret_sends_none() {
        // Google issues no secret at all for iOS and Android clients, so the
        // parameter has to be genuinely optional rather than defaulted.
        assert_eq!(
            value(&exchange_params("id", None, "c", "v", "r"), "client_secret"),
            None
        );
        assert_eq!(
            value(&refresh_params("id", None, "rt"), "client_secret"),
            None
        );
    }

    #[test]
    fn a_revoked_grant_is_its_own_error() {
        // It is the only failure the user has to act on, so it must not be
        // lumped in with transient ones — a retry loop on a revoked token
        // never recovers.
        let err = TokenError::NeedsReauth;
        assert!(err.to_string().contains("reconnect"));
    }
}
