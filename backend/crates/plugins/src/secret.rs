//! The webhook secret of a plugin and the signature of an event (ADR-0066).
//!
//! The secret is not stored: it is `HMAC-SHA256(plugins.secret_key, "vumlo-plugin:" + install id +
//! ":" + secret version)` in unpadded base64url, so it can be derived again whenever an event is
//! signed. A signature is `sha256=` and the lowercase hex of `HMAC-SHA256(secret, timestamp + "." +
//! body)`, the form a plugin written in any language can check.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use uuid::Uuid;
use vumlo_config::Secret;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
#[error("the key cannot be used to sign")]
pub struct SecretError;

/// `plugins.secret_key`, as bytes. Redacted in `Debug`.
#[derive(Clone)]
pub struct SecretKey(Vec<u8>);

impl SecretKey {
    /// `None` while the key is not set: no plugin can be installed then.
    pub fn from_config(secret: &Secret) -> Option<Self> {
        let key = secret.expose();
        (!key.is_empty()).then(|| Self(key.as_bytes().to_vec()))
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(***)")
    }
}

fn mac(key: &[u8]) -> Result<HmacSha256, SecretError> {
    HmacSha256::new_from_slice(key).map_err(|_| SecretError)
}

/// The secret of the install `install_id`, as it is on `secret_version`.
pub fn derive_secret(
    key: &SecretKey,
    install_id: Uuid,
    secret_version: i32,
) -> Result<String, SecretError> {
    let mut mac = mac(&key.0)?;
    mac.update(format!("vumlo-plugin:{install_id}:{secret_version}").as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// The value of `X-Vumlo-Signature` for `body` sent at `timestamp` (Unix seconds).
pub fn sign(secret: &str, timestamp: i64, body: &[u8]) -> Result<String, SecretError> {
    let mut mac = mac(secret.as_bytes())?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("sha256={hex}"))
}

#[cfg(test)]
mod tests;
