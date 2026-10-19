//! The session token in the operating system's keychain (ADR-0056): Keychain on macOS, Credential Manager
//! on Windows, the Secret Service on Linux. There is no file fallback: without a keychain the person cannot
//! sign in, and is told why, rather than getting a token in a plain file.

use std::sync::Mutex;

use keyring::Entry;

use crate::core::TokenStore;

const ACCOUNT: &str = "session";

pub struct KeychainToken {
    service: String,
    /// What was read or written, so a request does not ask the keychain every time.
    held: Mutex<Option<Option<String>>>,
}

impl KeychainToken {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_owned(),
            held: Mutex::new(None),
        }
    }

    fn entry(&self) -> keyring::Result<Entry> {
        Entry::new(&self.service, ACCOUNT)
    }
}

impl TokenStore for KeychainToken {
    fn get(&self) -> Option<String> {
        let mut held = self.held.lock().ok()?;
        held.get_or_insert_with(|| {
            self.entry()
                .ok()
                .and_then(|entry| entry.get_password().ok())
        })
        .clone()
    }

    fn set(&self, token: &str) -> Result<(), String> {
        self.entry()
            .and_then(|entry| entry.set_password(token))
            .map_err(|e| {
                format!("keychain: this device has no keychain that can hold the session ({e})")
            })?;
        if let Ok(mut held) = self.held.lock() {
            *held = Some(Some(token.to_owned()));
        }
        Ok(())
    }

    fn clear(&self) {
        if let Ok(entry) = self.entry() {
            // No entry is as good as a deleted one.
            let _ = entry.delete_credential();
        }
        if let Ok(mut held) = self.held.lock() {
            *held = Some(None);
        }
    }
}
