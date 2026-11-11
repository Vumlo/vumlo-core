//! The signed token of the one-click opt-out (ADR-0086): `base64url("v1|user|plugin|kind")`, a dot,
//! `base64url(HMAC-SHA256(key, "vumlo-mail-optout|" payload))`. Not stored, not expiring: an old
//! mail must still work. Deleting the signing key voids every link.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const DOMAIN: &[u8] = b"vumlo-mail-optout|";
const VERSION: &str = "v1";

/// What a token names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptOut {
    pub user_id: Uuid,
    pub plugin_id: String,
    pub kind: String,
}

fn mac(key: &[u8], payload: &[u8]) -> Option<HmacSha256> {
    let mut mac = HmacSha256::new_from_slice(key).ok()?;
    mac.update(DOMAIN);
    mac.update(payload);
    Some(mac)
}

/// The token for one (person, plugin, kind). `None` only for a key the MAC refuses, which a
/// 32-byte key never is.
pub fn sign(key: &[u8], opt_out: &OptOut) -> Option<String> {
    let payload = format!(
        "{VERSION}|{}|{}|{}",
        opt_out.user_id, opt_out.plugin_id, opt_out.kind
    );
    let tag = mac(key, payload.as_bytes())?.finalize().into_bytes();
    Some(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(tag)
    ))
}

/// What `token` names, if its signature holds and it is well formed. The comparison is constant
/// time.
pub fn verify(key: &[u8], token: &str) -> Option<OptOut> {
    let (payload, tag) = token.split_once('.')?;
    let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let tag = URL_SAFE_NO_PAD.decode(tag).ok()?;
    mac(key, &payload)?.verify_slice(&tag).ok()?;
    let payload = String::from_utf8(payload).ok()?;
    let mut parts = payload.split('|');
    let (version, user, plugin, kind) =
        (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if version != VERSION || parts.next().is_some() || plugin.is_empty() || kind.is_empty() {
        return None;
    }
    Some(OptOut {
        user_id: user.parse().ok()?,
        plugin_id: plugin.to_owned(),
        kind: kind.to_owned(),
    })
}

#[cfg(test)]
mod tests;
