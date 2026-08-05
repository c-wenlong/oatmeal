//! Holding the connection together: storage, refresh, and the one-shot flow.

use std::sync::Mutex;

use super::events;
use super::loopback::Loopback;
use super::pkce::{self, Callback, Pkce};
use super::token::{self, TokenError, Tokens};
use crate::detect::CalendarEvent;
use crate::llm::keys::KeyStore;

/// Keychain reference for the refresh token.
///
/// The same store as the LLM keys and the Notion token. A credential that can
/// read someone's calendar has no business in SQLite.
pub const REFRESH_TOKEN_KEY: &str = "google-calendar-refresh";

/// How long to wait for the browser before giving up.
///
/// Generous: the user may have to pick between several Google accounts, and a
/// flow that times out mid-consent is worse than one that waits.
pub const FLOW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// A live connection to Google Calendar.
///
/// The access token lives here and nowhere else — in memory, for the hour it is
/// good for. Only the refresh token is persisted.
#[derive(Default)]
pub struct Connection {
    tokens: Mutex<Option<Tokens>>,
}

impl Connection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a refresh token is stored. Not whether it still works — that is
    /// only knowable by using it.
    pub fn is_connected(&self, keys: &dyn KeyStore) -> bool {
        keys.has(REFRESH_TOKEN_KEY)
    }

    /// Forgets the connection.
    ///
    /// Clears the in-memory access token too. Leaving it would keep the
    /// integration working for up to an hour after the user disconnected,
    /// which is exactly the kind of thing that makes a disconnect button feel
    /// like a lie.
    pub fn disconnect(&self, keys: &dyn KeyStore) -> Result<(), TokenError> {
        if let Ok(mut tokens) = self.tokens.lock() {
            *tokens = None;
        }
        keys.delete(REFRESH_TOKEN_KEY)
            .map_err(|e| TokenError::Rejected(e.to_string()))
    }

    /// A usable access token, refreshing if the cached one is stale.
    pub async fn access_token(
        &self,
        http: &reqwest::Client,
        keys: &dyn KeyStore,
        client_id: &str,
        now_ms: i64,
    ) -> Result<String, TokenError> {
        if let Ok(guard) = self.tokens.lock() {
            if let Some(tokens) = guard.as_ref() {
                if !tokens.is_expired(now_ms) {
                    return Ok(tokens.access_token.clone());
                }
            }
        }

        let refresh_token = keys
            .get(REFRESH_TOKEN_KEY)
            .map_err(|e| TokenError::Rejected(e.to_string()))?
            .ok_or(TokenError::NotConnected)?;

        let refreshed = token::refresh(http, client_id, &refresh_token, now_ms).await?;
        let access = refreshed.access_token.clone();
        if let Ok(mut guard) = self.tokens.lock() {
            *guard = Some(refreshed);
        }
        Ok(access)
    }

    /// Stores the tokens from a completed authorization.
    pub fn adopt(&self, keys: &dyn KeyStore, tokens: Tokens) -> Result<(), TokenError> {
        if let Some(refresh) = &tokens.refresh_token {
            keys.set(REFRESH_TOKEN_KEY, refresh)
                .map_err(|e| TokenError::Rejected(e.to_string()))?;
        }
        if let Ok(mut guard) = self.tokens.lock() {
            *guard = Some(tokens);
        }
        Ok(())
    }

    /// Upcoming meeting-shaped events, ready for the detection queue.
    pub async fn upcoming(
        &self,
        http: &reqwest::Client,
        keys: &dyn KeyStore,
        client_id: &str,
        now_ms: i64,
        horizon_ms: i64,
    ) -> Result<Vec<CalendarEvent>, TokenError> {
        let access = self.access_token(http, keys, client_id, now_ms).await?;
        events::upcoming(http, &access, now_ms, horizon_ms).await
    }
}

/// What a completed flow produced, minus anything secret.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowOutcome {
    pub connected: bool,
    /// Why not, when it did not work.
    pub reason: Option<String>,
}

/// Runs the whole authorization, start to finish.
///
/// Blocking, and called from a blocking thread: the loopback listener is a
/// plain `TcpListener`, and the flow is a single linear sequence that reads far
/// worse split across an async boundary for no gain.
///
/// `open_browser` is handed the **finished URL**. The PKCE pair and the state
/// are generated in here and never leave — a caller that had to build the URL
/// would need the verifier, and the verifier's whole value is that it stays in
/// the process that started the flow.
pub fn authorize(
    client_id: &str,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
    exchange: impl FnOnce(&str, &str, &str) -> Result<Tokens, TokenError>,
) -> Result<Tokens, TokenError> {
    let loopback = Loopback::bind().map_err(|e| TokenError::Rejected(e.to_string()))?;
    let redirect_uri = loopback.redirect_uri();

    let pkce = Pkce::generate();
    let state = pkce::random_state();

    let url = pkce::authorize_url(client_id, &redirect_uri, &pkce, &state);
    open_browser(&url).map_err(TokenError::Rejected)?;

    let callback = loopback
        .wait(FLOW_TIMEOUT)
        .map_err(|e| TokenError::Rejected(e.to_string()))?;

    match callback {
        Callback::Denied { error, .. } => Err(TokenError::Rejected(error)),
        Callback::Code {
            code,
            state: returned,
        } => {
            // The check that makes `state` worth having: without it, anything
            // able to reach the loopback port could hand us a code belonging to
            // a different account and we would redeem it.
            if !pkce::state_matches(&state, &returned) {
                return Err(TokenError::Rejected(
                    "the browser came back with a mismatched state — the request did not \
                     come from Oatmeal"
                        .into(),
                ));
            }
            exchange(&code, &pkce.verifier, &redirect_uri)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::keys::MemoryKeyStore;

    /// Pulls the loopback port back out of the authorization URL, which is the
    /// only thing the browser callback gets handed.
    fn redirect_port(url: &str) -> String {
        let encoded = url
            .split("redirect_uri=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap();
        encoded.rsplit("%3A").next().unwrap().to_string()
    }

    fn tokens(expires_at_ms: i64) -> Tokens {
        Tokens {
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            expires_at_ms,
        }
    }

    #[test]
    fn a_fresh_app_is_not_connected() {
        let keys = MemoryKeyStore::default();
        assert!(!Connection::new().is_connected(&keys));
    }

    #[test]
    fn adopting_tokens_stores_only_the_refresh_token() {
        // The access token is good for an hour; persisting it would put a
        // bearer token on disk to save one round trip.
        let keys = MemoryKeyStore::default();
        let connection = Connection::new();
        connection.adopt(&keys, tokens(i64::MAX)).unwrap();

        assert!(connection.is_connected(&keys));
        assert_eq!(keys.get(REFRESH_TOKEN_KEY).unwrap().as_deref(), Some("rt"));
        // And nothing else was written.
        assert_eq!(keys.get("google-calendar-access").unwrap(), None);
    }

    #[test]
    fn disconnecting_clears_the_cached_access_token_too() {
        // Otherwise the integration keeps working for up to an hour after the
        // user disconnected, which makes the button a lie.
        let keys = MemoryKeyStore::default();
        let connection = Connection::new();
        connection.adopt(&keys, tokens(i64::MAX)).unwrap();

        connection.disconnect(&keys).unwrap();

        assert!(!connection.is_connected(&keys));
        assert!(connection.tokens.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_cached_token_is_reused_while_it_is_good() {
        // No network call: an unreachable client proves nothing was attempted.
        let keys = MemoryKeyStore::default();
        let connection = Connection::new();
        connection.adopt(&keys, tokens(1_000_000)).unwrap();

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1))
            .build()
            .unwrap();
        let token = connection
            .access_token(&http, &keys, "cid", 0)
            .await
            .unwrap();
        assert_eq!(token, "at");
    }

    #[tokio::test]
    async fn no_stored_token_reports_not_connected() {
        let keys = MemoryKeyStore::default();
        let connection = Connection::new();
        let http = reqwest::Client::new();

        let err = connection
            .access_token(&http, &keys, "cid", 0)
            .await
            .unwrap_err();
        assert!(matches!(err, TokenError::NotConnected));
    }

    #[test]
    fn a_mismatched_state_stops_the_flow_before_the_exchange() {
        // The attack: something reaches the loopback port and offers a code
        // from another account. It must never be redeemed.
        let exchanged = std::cell::Cell::new(false);

        let result = authorize(
            "client-id",
            |url| {
                // Simulate the browser coming back immediately with a bad state
                // by driving the loopback ourselves.
                let redirect = redirect_port(url);
                std::thread::spawn(move || {
                    use std::io::{Read, Write};
                    if let Ok(mut stream) =
                        std::net::TcpStream::connect(format!("127.0.0.1:{redirect}"))
                    {
                        let _ = stream.write_all(
                            b"GET /?code=stolen&state=not-ours HTTP/1.1\r\nHost: x\r\n\r\n",
                        );
                        let mut sink = String::new();
                        let _ = stream.read_to_string(&mut sink);
                    }
                });
                Ok(())
            },
            |_code, _verifier, _redirect| {
                exchanged.set(true);
                Ok(tokens(0))
            },
        );

        assert!(result.is_err(), "a mismatched state was accepted");
        assert!(!exchanged.get(), "the stolen code was redeemed");
        assert!(result.unwrap_err().to_string().contains("mismatched state"));
    }

    #[test]
    fn a_denial_ends_the_flow_without_an_exchange() {
        let exchanged = std::cell::Cell::new(false);

        let result = authorize(
            "client-id",
            |url| {
                let redirect = redirect_port(url);
                std::thread::spawn(move || {
                    use std::io::{Read, Write};
                    if let Ok(mut stream) =
                        std::net::TcpStream::connect(format!("127.0.0.1:{redirect}"))
                    {
                        let _ = stream.write_all(
                            b"GET /?error=access_denied&state=s HTTP/1.1\r\nHost: x\r\n\r\n",
                        );
                        let mut sink = String::new();
                        let _ = stream.read_to_string(&mut sink);
                    }
                });
                Ok(())
            },
            |_code, _verifier, _redirect| {
                exchanged.set(true);
                Ok(tokens(0))
            },
        );

        assert!(result.is_err());
        assert!(!exchanged.get());
    }

    #[test]
    fn a_browser_that_will_not_open_fails_immediately() {
        // Rather than sitting on a loopback port for five minutes.
        let result = authorize(
            "client-id",
            |_| Err("no browser".into()),
            |_, _, _| Ok(tokens(0)),
        );
        assert!(result.is_err());
    }
}
