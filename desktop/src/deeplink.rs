//! `vumlo-<abbr>://...` links (ADR-0056): a link from outside the app names a screen, and only the screens
//! listed here. Whatever else a link says is dropped, so a link cannot make the window go anywhere else.

const SCHEME: &str = env!("VUMLO_URL_SCHEME");

pub fn scheme() -> &'static str {
    SCHEME
}

/// A UUID as the server writes ids: 36 characters of hex and dashes.
fn is_id(value: &str) -> bool {
    value.len() == 36 && value.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// The address inside the app for a link, `None` for anything that is not one of ours.
pub fn route_of(link: &str, scheme: &str) -> Option<String> {
    let rest = link.strip_prefix(scheme)?.strip_prefix("://")?;
    let rest = rest.split('#').next()?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    match path.trim_end_matches('/') {
        "" | "home" => Some("/".to_owned()),
        "courses" | "calendar" | "grades" => Some(format!("/{}", path.trim_end_matches('/'))),
        "course" => param(query, "id")
            .filter(|id| is_id(id))
            .map(|id| format!("/course?id={id}")),
        "item" => {
            let course = param(query, "course").filter(|id| is_id(id))?;
            let item = param(query, "id").filter(|id| is_id(id))?;
            Some(format!("/course/item?id={course}&item={item}"))
        }
        _ => None,
    }
}

/// The code of a `vumlo-<abbr>://auth/google?code=...` link: what the web app's sign-in page hands to the app
/// (ADR-0062). Only the characters a code is made of; anything else is not a code.
pub fn google_code(link: &str, scheme: &str) -> Option<String> {
    let rest = link.strip_prefix(scheme)?.strip_prefix("://")?;
    let (path, query) = rest.split_once('?')?;
    if path.trim_end_matches('/') != "auth/google" {
        return None;
    }
    param(query.split('#').next()?, "code")
        .filter(|code| (1..=128).contains(&code.len()))
        .filter(|code| {
            code.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests;
