//! API key storage.
//!
//! Keys go in the macOS Keychain and nowhere else — never SQLite, never the
//! config, never a log line. The trait exists so tests exercise the real call
//! sites against an in-memory store instead of writing to (and needing an
//! unlocked) system keychain in CI.

use std::collections::HashMap;
use std::sync::Mutex;

const SERVICE: &str = "com.kaichen.oatmeal";

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("keychain: {0}")]
    Backend(String),
}

pub trait KeyStore: Send + Sync {
    fn set(&self, reference: &str, secret: &str) -> Result<(), KeyError>;
    /// `Ok(None)` when no key was ever stored, which is different from an error.
    fn get(&self, reference: &str) -> Result<Option<String>, KeyError>;
    fn delete(&self, reference: &str) -> Result<(), KeyError>;
    fn has(&self, reference: &str) -> bool {
        matches!(self.get(reference), Ok(Some(_)))
    }
}

pub struct Keychain;

impl KeyStore for Keychain {
    fn set(&self, reference: &str, secret: &str) -> Result<(), KeyError> {
        keyring::Entry::new(SERVICE, reference)
            .and_then(|entry| entry.set_password(secret))
            .map_err(|e| KeyError::Backend(e.to_string()))
    }

    fn get(&self, reference: &str) -> Result<Option<String>, KeyError> {
        let entry = keyring::Entry::new(SERVICE, reference)
            .map_err(|e| KeyError::Backend(e.to_string()))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            // "Never stored" is a normal state, not a failure.
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyError::Backend(e.to_string())),
        }
    }

    fn delete(&self, reference: &str) -> Result<(), KeyError> {
        let entry = keyring::Entry::new(SERVICE, reference)
            .map_err(|e| KeyError::Backend(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeyError::Backend(e.to_string())),
        }
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryKeyStore {
    entries: Mutex<HashMap<String, String>>,
}

impl KeyStore for MemoryKeyStore {
    fn set(&self, reference: &str, secret: &str) -> Result<(), KeyError> {
        self.entries
            .lock()
            .map_err(|_| KeyError::Backend("poisoned".into()))?
            .insert(reference.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, reference: &str) -> Result<Option<String>, KeyError> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| KeyError::Backend("poisoned".into()))?
            .get(reference)
            .cloned())
    }

    fn delete(&self, reference: &str) -> Result<(), KeyError> {
        self.entries
            .lock()
            .map_err(|_| KeyError::Backend("poisoned".into()))?
            .remove(reference);
        Ok(())
    }
}

/// Redacts a key for display. The last four characters are enough to tell two
/// keys apart without putting the secret on screen or in a screenshot.
pub fn redact(secret: &str) -> String {
    let visible: String = secret
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if secret.chars().count() <= 4 {
        "••••".to_string()
    } else {
        format!("••••{visible}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_key() {
        let store = MemoryKeyStore::default();
        store.set("openai", "sk-secret").unwrap();
        assert_eq!(store.get("openai").unwrap().as_deref(), Some("sk-secret"));
        assert!(store.has("openai"));
    }

    #[test]
    fn a_missing_key_is_none_not_an_error() {
        // Callers branch on "configured yet?", which must not require catching
        // an error to answer.
        let store = MemoryKeyStore::default();
        assert_eq!(store.get("never-set").unwrap(), None);
        assert!(!store.has("never-set"));
    }

    #[test]
    fn deleting_removes_the_key() {
        let store = MemoryKeyStore::default();
        store.set("openai", "sk-secret").unwrap();
        store.delete("openai").unwrap();
        assert_eq!(store.get("openai").unwrap(), None);
    }

    #[test]
    fn deleting_a_key_that_was_never_set_is_not_an_error() {
        // "Remove my key" should succeed whether or not one was there.
        assert!(MemoryKeyStore::default().delete("nope").is_ok());
    }

    #[test]
    fn keys_are_scoped_by_reference() {
        let store = MemoryKeyStore::default();
        store.set("openai", "sk-a").unwrap();
        store.set("anthropic", "sk-b").unwrap();
        assert_eq!(store.get("openai").unwrap().as_deref(), Some("sk-a"));
        assert_eq!(store.get("anthropic").unwrap().as_deref(), Some("sk-b"));
    }

    #[test]
    fn overwriting_replaces_rather_than_appends() {
        let store = MemoryKeyStore::default();
        store.set("openai", "old").unwrap();
        store.set("openai", "new").unwrap();
        assert_eq!(store.get("openai").unwrap().as_deref(), Some("new"));
    }

    #[test]
    fn redaction_never_reveals_the_secret() {
        let secret = "sk-proj-abcdefghijklmnop1234";
        let shown = redact(secret);
        assert!(!shown.contains("abcdefghijklmnop"));
        assert!(shown.ends_with("1234"));
        assert!(shown.starts_with('•'));
    }

    #[test]
    fn a_short_key_is_redacted_entirely() {
        // Showing "the last four" of a five-character key reveals most of it.
        assert_eq!(redact("abcd"), "••••");
        assert_eq!(redact(""), "••••");
    }
}
