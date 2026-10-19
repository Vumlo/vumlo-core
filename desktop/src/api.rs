//! The only way the webview reaches the server: a request made here, in Rust, so no CORS, no CSRF
//! Origin check and no token in JavaScript (ADR-0056).

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;

const API_URL: &str = env!("VUMLO_API_URL");

static TOKEN: Mutex<Option<String>> = Mutex::new(None);

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("vumlo-desktop/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("the HTTP client")
    })
}

#[derive(Serialize)]
pub struct Reply {
    status: u16,
    body: Option<Value>,
}

/// A path the webview may ask for: absolute, no way out of `/api/v1`, no scheme or host.
fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains("..")
        && !path.contains('\\')
        && !path.contains("://")
}

#[tauri::command]
pub async fn api(method: String, path: String, body: Option<Value>) -> Result<Reply, String> {
    if !valid_path(&path) {
        return Err(format!("refused path {path}"));
    }
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
    let mut req = client().request(method, format!("{API_URL}{path}"));
    if let Some(token) = TOKEN.lock().unwrap().clone() {
        req = req.bearer_auth(token);
    }
    if let Some(body) = &body {
        req = req.json(body);
    }
    let res = req.send().await.map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    let text = res.text().await.map_err(|e| e.to_string())?;
    let mut json = if text.is_empty() {
        None
    } else {
        serde_json::from_str(&text).ok()
    };
    if path == "/auth/logout" {
        // Whatever the server says, this device no longer holds a session.
        *TOKEN.lock().unwrap() = None;
    }
    if path == "/auth/login" && status == 200 {
        // The token stays in Rust; the webview gets the user (ADR-0056).
        if let Some(Value::Object(map)) = &mut json {
            if let Some(Value::String(token)) = map.remove("token") {
                *TOKEN.lock().unwrap() = Some(token);
            }
            if let Some(user) = map.remove("user") {
                json = Some(user);
            }
        }
    }
    Ok(Reply { status, body: json })
}

#[cfg(test)]
mod tests {
    use super::valid_path;

    #[test]
    fn only_paths_inside_the_api_are_accepted() {
        assert!(valid_path("/me"));
        assert!(valid_path("/courses/abc/content?x=1"));
        for bad in [
            "me",
            "//evil.example/x",
            "/../x",
            "/a/../b",
            "/a\\b",
            "/x://y",
            "",
        ] {
            assert!(!valid_path(bad), "{bad}");
        }
    }
}
