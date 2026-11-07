//! The webhook secret of a plugin, the signature of an event (ADR-0066), and the sealing of a
//! secret setting (ADR-0072, ADR-0073).
//!
//! The secret is not stored: it is `HMAC-SHA256(plugins.secret_key, "vumlo-plugin:" + install id +
//! ":" + secret version)` in unpadded base64url, so it can be derived again whenever an event is
//! signed. A signature is `sha256=` and the lowercase hex of `HMAC-SHA256(secret, timestamp + "." +
//! body)`, the form a plugin written in any language can check.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, KeyInit, Mac};
use rand::Rng;
use sha2::{Digest, Sha256};
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
    Ok(format!("sha256={}", hex(&mac.finalize().into_bytes())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The first byte of a sealed value. A later format (another cipher, another key) gets the next
/// number, and `open` tells them apart by it.
const SEALED_V1: u8 = 1;
const NONCE_LEN: usize = 24;

/// The XChaCha20-Poly1305 key: `HMAC-SHA256(plugins.secret_key, label)`, so that this key and the
/// bytes that derive webhook secrets are not the same.
fn seal_cipher(key: &SecretKey) -> Result<XChaCha20Poly1305, SecretError> {
    let mut mac = mac(&key.0)?;
    mac.update(b"vumlo-setting-seal-v1");
    Ok(XChaCha20Poly1305::new(&mac.finalize().into_bytes()))
}

/// What the tag covers besides the ciphertext: the format, the install and the setting, so a
/// sealed value cannot be moved to another setting or install, or passed off as another format.
fn associated_data(install_id: Uuid, name: &str) -> Vec<u8> {
    let mut data = vec![SEALED_V1];
    data.extend_from_slice(install_id.as_bytes());
    data.extend_from_slice(name.as_bytes());
    data
}

/// Seals `plain` for the setting `name` of the install with XChaCha20-Poly1305 (ADR-0073) and a
/// random nonce. The result is unpadded base64url of `version || nonce || ciphertext || tag`.
pub fn seal(
    key: &SecretKey,
    install_id: Uuid,
    name: &str,
    plain: &str,
) -> Result<String, SecretError> {
    let mut nonce = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce);
    let cipher = seal_cipher(key)?;
    let ciphertext = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plain.as_bytes(),
                aad: &associated_data(install_id, name),
            },
        )
        .map_err(|_| SecretError)?;
    let mut sealed = vec![SEALED_V1];
    sealed.extend_from_slice(&nonce);
    sealed.extend(ciphertext);
    Ok(URL_SAFE_NO_PAD.encode(sealed))
}

/// The inverse of [`seal`]; `SecretError` for a value that was changed, moved to another setting
/// or install, of an unknown format, or sealed with another key.
pub fn open(
    key: &SecretKey,
    install_id: Uuid,
    name: &str,
    sealed: &str,
) -> Result<String, SecretError> {
    let bytes = URL_SAFE_NO_PAD.decode(sealed).map_err(|_| SecretError)?;
    let (&version, rest) = bytes.split_first().ok_or(SecretError)?;
    if version != SEALED_V1 || rest.len() < NONCE_LEN {
        return Err(SecretError);
    }
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| SecretError)?;
    let plain = seal_cipher(key)?
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: ciphertext,
                aad: &associated_data(install_id, name),
            },
        )
        .map_err(|_| SecretError)?;
    String::from_utf8(plain).map_err(|_| SecretError)
}

/// How long a launch token lives.
pub const LAUNCH_TTL_SECONDS: i64 = 60;

/// What a launch token says: who looks at which assignment, in the terms of its course.
pub struct LaunchClaims<'a> {
    pub plugin: &'a str,
    pub assignment: Uuid,
    pub user: Uuid,
    /// `student` or `staff`.
    pub role: &'a str,
    /// Unix seconds. The token expires [`LAUNCH_TTL_SECONDS`] later.
    pub issued_at: i64,
}

/// `base64url(payload) + "." + hex(HMAC-SHA256(secret, "vumlo-launch." + base64url(payload)))`.
/// The prefix keeps the signature apart from that of an event (`timestamp.body`), so a signed event
/// cannot be passed off as a token or the other way round. The plugin's server checks it with the
/// secret it has.
pub fn launch_token(secret: &str, claims: &LaunchClaims<'_>) -> Result<String, SecretError> {
    let payload = serde_json::json!({
        "v": 1,
        "plugin": claims.plugin,
        "assignment": claims.assignment,
        "user": claims.user,
        "role": claims.role,
        "iat": claims.issued_at,
        "exp": claims.issued_at + LAUNCH_TTL_SECONDS,
    });
    let body = URL_SAFE_NO_PAD.encode(payload.to_string());
    let mut mac = mac(secret.as_bytes())?;
    mac.update(b"vumlo-launch.");
    mac.update(body.as_bytes());
    Ok(format!("{body}.{}", hex(&mac.finalize().into_bytes())))
}

/// What the core tells the plugin's server about a call a person made through it (ADR-0078).
pub struct ProxyClaims<'a> {
    pub method: &'a str,
    /// The request-target the plugin receives: path and query, as sent.
    pub target: &'a str,
    pub user: Uuid,
    /// The institution role of the user.
    pub role: &'a str,
    pub course: Option<Uuid>,
    /// The role of the user in `course`, when they have one.
    pub course_role: Option<&'a str>,
    /// Fresh for every request; the plugin refuses one it has seen.
    pub nonce: Uuid,
    /// Unix seconds; the plugin refuses a call more than 300 seconds off.
    pub timestamp: i64,
}

/// The value of `X-Vumlo-Signature` of a proxied call: `sha256=` and the hex HMAC-SHA256, under the
/// plugin's secret, of `timestamp + "." + canonical`, where canonical is these lines joined by `\n`:
/// `vumlo-proxy`, nonce, method, target, user, role, course (`-` for none), course role (`-` for
/// none) and the hex SHA-256 of the body. The first line keeps it apart from an event and a launch
/// token, which are signed with the same secret; every other line is what the call is about, so none
/// of it can be changed on the way.
pub fn proxy_signature(
    secret: &str,
    claims: &ProxyClaims<'_>,
    body: &[u8],
) -> Result<String, SecretError> {
    let canonical = format!(
        "vumlo-proxy\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        claims.nonce,
        claims.method,
        claims.target,
        claims.user,
        claims.role,
        claims
            .course
            .map_or_else(|| "-".to_owned(), |c| c.to_string()),
        claims.course_role.unwrap_or("-"),
        hex(&Sha256::digest(body)),
    );
    sign(secret, claims.timestamp, canonical.as_bytes())
}

#[cfg(test)]
mod tests;
