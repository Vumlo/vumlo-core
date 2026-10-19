//! Everything the desktop app does with the network (ADR-0056, ADR-0060): a read-through cache for `GET`,
//! a queue for the writes a student makes without a connection, and the session. The webview reaches it
//! through the commands of `api.rs` and nowhere else.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::store::{OutboxRow, Store};

/// Where the session token lives. The app keeps it in the OS keychain; tests keep it in memory.
pub trait TokenStore: Send + Sync {
    fn get(&self) -> Option<String>;
    fn set(&self, token: &str) -> Result<(), String>;
    fn clear(&self);
}

/// The token of a test: held in memory only.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryToken(std::sync::Mutex<Option<String>>);

#[cfg(test)]
impl TokenStore for MemoryToken {
    fn get(&self) -> Option<String> {
        self.0.lock().ok()?.clone()
    }
    fn set(&self, token: &str) -> Result<(), String> {
        *self.0.lock().map_err(|e| e.to_string())? = Some(token.to_owned());
        Ok(())
    }
    fn clear(&self) {
        if let Ok(mut held) = self.0.lock() {
            *held = None;
        }
    }
}

/// What the app tells the window about the connection and the queue (the `sync` event).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SyncState {
    pub online: bool,
    /// Writes waiting to be sent, being sent, or waiting for a back-off.
    pub pending: usize,
    /// Writes the server refused for good; they wait for the person to read them and discard them.
    pub failed: usize,
    /// The server no longer accepts the session: nothing is sent until the person signs in again.
    pub paused: bool,
    /// Unix milliseconds of the last answer from the server.
    pub last_ok: Option<i64>,
    /// How many queued writes the server has accepted since the app started.
    pub sent: u64,
    /// The paths of the writes that wait, so a screen can say "waiting to send".
    pub queued: Vec<String>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Reply {
    pub status: u16,
    pub body: Option<Value>,
    /// The server did not answer; this is the last copy of it.
    pub from_cache: bool,
    /// Unix milliseconds the copy was last confirmed by the server.
    pub cached_at: Option<i64>,
    /// The write is in the queue; `body` is what the window shows meanwhile, if there is anything.
    pub queued: bool,
}

impl Reply {
    fn plain(status: u16, body: Option<Value>) -> Self {
        Self {
            status,
            body,
            from_cache: false,
            cached_at: None,
            queued: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    pub body: Option<Value>,
    pub queue: bool,
    pub idem_key: Option<String>,
}

struct Wire {
    status: u16,
    etag: Option<String>,
    retry_after: Option<u64>,
    text: String,
}

pub struct Core {
    store: Store,
    http: reqwest::Client,
    api_url: String,
    token: Arc<dyn TokenStore>,
    emit: Box<dyn Fn(&SyncState) + Send + Sync>,
    wake: Notify,
    online: AtomicBool,
    paused: AtomicBool,
    last_ok: AtomicI64,
    sent: AtomicU64,
    /// The secret of the sign-in with Google that is waiting for its link (ADR-0062).
    google: std::sync::Mutex<Option<String>>,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A path the window may ask for: absolute, no way out of `/api/v1`, no scheme or host.
pub fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains("..")
        && !path.contains('\\')
        && !path.contains("://")
}

/// Seconds before a write is tried again: 5, 10, 20 ... up to five minutes.
pub fn backoff_secs(attempts: i64) -> i64 {
    let exponent = (attempts - 1).clamp(0, 6) as u32;
    (5_i64 << exponent).min(300)
}

/// A server that could not do the work now, and may when asked again.
fn retryable(status: u16) -> bool {
    status == 408 || status == 429 || status >= 500
}

/// The `GET`s worth keeping. Sign-in and health are about the moment, not the person's data.
fn cacheable(path: &str) -> bool {
    !path.starts_with("/auth/") && !path.starts_with("/health")
}

/// `1970-01-01T00:00:00Z` for unix milliseconds. The queue shows a draft's save time before the server has
/// seen it, in the form the server uses.
pub fn rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// `bytes` random bytes as lowercase hex.
fn random_hex(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0_u8; bytes];
    getrandom::fill(&mut buffer).map_err(|e| format!("no source of randomness: {e}"))?;
    Ok(hex(&buffer))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The SHA-256 of `text` as 64 lowercase hex characters, the form the server takes as an `app_challenge`.
pub fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(text.as_bytes()))
}

fn parse(text: &str) -> Option<Value> {
    if text.is_empty() {
        None
    } else {
        serde_json::from_str(text).ok()
    }
}

impl Core {
    pub fn new(
        store: Store,
        api_url: String,
        token: Arc<dyn TokenStore>,
        emit: Box<dyn Fn(&SyncState) + Send + Sync>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("vumlo-desktop/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            store,
            http,
            api_url,
            token,
            emit,
            wake: Notify::new(),
            online: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            last_ok: AtomicI64::new(0),
            sent: AtomicU64::new(0),
            google: std::sync::Mutex::new(None),
        }
    }

    pub fn state(&self) -> SyncState {
        let rows = self.store.outbox_list().unwrap_or_default();
        let waiting = |row: &&OutboxRow| row.state != "failed";
        let last_ok = self.last_ok.load(Ordering::Relaxed);
        SyncState {
            online: self.online.load(Ordering::Relaxed),
            pending: rows.iter().filter(waiting).count(),
            failed: rows.iter().filter(|row| row.state == "failed").count(),
            paused: self.paused.load(Ordering::Relaxed),
            last_ok: (last_ok > 0).then_some(last_ok),
            sent: self.sent.load(Ordering::Relaxed),
            queued: rows
                .iter()
                .filter(waiting)
                .map(|row| row.path.clone())
                .collect(),
        }
    }

    fn publish(&self) {
        (self.emit)(&self.state());
    }

    fn set_online(&self, up: bool) {
        let was = self.online.swap(up, Ordering::Relaxed);
        if up {
            self.last_ok.store(now_ms(), Ordering::Relaxed);
        }
        if up != was {
            self.wake.notify_one();
            self.publish();
        }
    }

    // --- the one door ----------------------------------------------------------------------------

    pub async fn request(&self, request: ApiRequest) -> Result<Reply, String> {
        if !valid_path(&request.path) {
            return Err(format!("refused path {}", request.path));
        }
        let method = request.method.to_ascii_uppercase();
        if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
            return Err(format!("refused method {method}"));
        }
        match (method.as_str(), request.path.as_str()) {
            ("GET", path) => self.read(path).await,
            ("POST", "/auth/login") => self.login(request.body).await,
            ("POST", "/auth/logout") => Ok(self.logout().await),
            _ if request.queue => self.write_queued(&method, request).await,
            _ => {
                self.write(&method, &request.path, request.body.as_ref())
                    .await
            }
        }
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<String>,
        headers: &[(&str, &str)],
    ) -> Result<Wire, reqwest::Error> {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
        let mut builder = self.http.request(method, format!("{}{path}", self.api_url));
        if let Some(token) = self.token.get() {
            builder = builder.bearer_auth(token);
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        if let Some(body) = body {
            builder = builder
                .header("content-type", "application/json")
                .body(body);
        }
        let response = builder.send().await?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let etag = header("etag");
        let retry_after = header("retry-after").and_then(|v| v.parse().ok());
        let text = response.text().await?;
        Ok(Wire {
            status,
            etag,
            retry_after,
            text,
        })
    }

    // --- reading ---------------------------------------------------------------------------------

    async fn read(&self, path: &str) -> Result<Reply, String> {
        if !cacheable(path) {
            return self.write("GET", path, None).await;
        }
        let cached = self.store.cache_get(path).map_err(|e| e.to_string())?;
        let tag = cached.as_ref().and_then(|c| c.etag.clone());
        let headers: Vec<(&str, &str)> = tag
            .as_deref()
            .map(|t| ("if-none-match", t))
            .into_iter()
            .collect();
        let copy = |entry: &crate::store::CacheEntry| Reply {
            status: 200,
            body: parse(&entry.body),
            from_cache: true,
            cached_at: Some(entry.fetched_at),
            queued: false,
        };
        match self.send("GET", path, None, &headers).await {
            Ok(wire) if wire.status == 200 => {
                self.set_online(true);
                self.store
                    .cache_put(path, &wire.text, wire.etag.as_deref(), now_ms())
                    .map_err(|e| e.to_string())?;
                Ok(Reply::plain(200, parse(&wire.text)))
            }
            Ok(wire) if wire.status == 304 && cached.is_some() => {
                self.set_online(true);
                self.store
                    .cache_touch(path, now_ms())
                    .map_err(|e| e.to_string())?;
                let entry = cached.as_ref().ok_or("no copy")?;
                Ok(Reply::plain(200, parse(&entry.body)))
            }
            Ok(wire) if wire.status >= 500 && cached.is_some() => {
                self.set_online(false);
                cached
                    .as_ref()
                    .map(copy)
                    .ok_or_else(|| "no copy".to_owned())
            }
            Ok(wire) => {
                self.set_online(true);
                // Gone or no longer allowed: the server wins, the copy goes.
                if matches!(wire.status, 403 | 404) {
                    self.store.cache_delete(path).map_err(|e| e.to_string())?;
                }
                Ok(Reply::plain(wire.status, parse(&wire.text)))
            }
            Err(error) => {
                self.set_online(false);
                match cached.as_ref() {
                    Some(entry) => Ok(copy(entry)),
                    None => Err(error.to_string()),
                }
            }
        }
    }

    // --- writing ---------------------------------------------------------------------------------

    async fn write(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Reply, String> {
        let wire = self
            .send(method, path, body.map(Value::to_string), &[])
            .await
            .inspect_err(|_| self.set_online(false))
            .map_err(|e| e.to_string())?;
        self.set_online(true);
        Ok(Reply::plain(wire.status, parse(&wire.text)))
    }

    /// A write that waits for a connection instead of failing. It is tried at once, unless writes are
    /// already waiting (their order is kept); when the server cannot be reached it joins the queue.
    async fn write_queued(&self, method: &str, request: ApiRequest) -> Result<Reply, String> {
        let key = request
            .idem_key
            .ok_or("a queued write needs an idempotency key")?;
        let text = request.body.as_ref().map(Value::to_string);
        if !self.store.has_waiting().map_err(|e| e.to_string())? {
            match self
                .send(
                    method,
                    &request.path,
                    text.clone(),
                    &[("idempotency-key", &key)],
                )
                .await
            {
                Ok(wire) if !retryable(wire.status) => {
                    self.set_online(true);
                    return Ok(Reply::plain(wire.status, parse(&wire.text)));
                }
                Ok(_) | Err(_) => self.set_online(false),
            }
        }
        self.store
            .enqueue(method, &request.path, text.as_deref(), &key, now_ms())
            .map_err(|e| e.to_string())?;
        let shown = self.optimistic(method, &request.path, request.body.as_ref());
        self.wake.notify_one();
        self.publish();
        Ok(Reply {
            status: if shown.is_some() { 200 } else { 202 },
            body: shown,
            from_cache: false,
            cached_at: None,
            queued: true,
        })
    }

    /// What a student sees of a draft that only this device has: the cached submission gets it, so a
    /// reload shows the text, and it carries no tag, so the next online read replaces it.
    fn optimistic(&self, method: &str, path: &str, body: Option<&Value>) -> Option<Value> {
        let id = path
            .strip_prefix("/assignments/")
            .and_then(|rest| rest.strip_suffix("/submission/draft"))?;
        if method != "PUT" {
            return None;
        }
        let draft = json!({
            "content": body?.get("content")?,
            "updated_at": rfc3339(now_ms()),
        });
        let key = format!("/assignments/{id}/submission");
        if let Ok(Some(entry)) = self.store.cache_get(&key)
            && let Some(Value::Object(mut submission)) = parse(&entry.body)
        {
            submission.insert("draft".to_owned(), draft.clone());
            let patched = Value::Object(submission).to_string();
            let _ = self.store.cache_put(&key, &patched, None, entry.fetched_at);
        }
        Some(draft)
    }

    // --- the session -----------------------------------------------------------------------------

    async fn login(&self, body: Option<Value>) -> Result<Reply, String> {
        let wire = self
            .send(
                "POST",
                "/auth/login",
                body.as_ref().map(Value::to_string),
                &[],
            )
            .await
            .inspect_err(|_| self.set_online(false))
            .map_err(|e| e.to_string())?;
        self.set_online(true);
        let mut json = parse(&wire.text);
        if wire.status == 200 {
            json = Some(self.accept_session(json)?);
        }
        Ok(Reply::plain(wire.status, json))
    }

    /// Takes the `{token, user}` of a sign-in: the token goes to the keychain, the user is returned. Another
    /// person on this device starts with nothing of the last one's (ADR-0060).
    fn accept_session(&self, answer: Option<Value>) -> Result<Value, String> {
        let Some(Value::Object(mut map)) = answer else {
            return Err("the server did not answer with a session".to_owned());
        };
        let (Some(Value::String(token)), Some(user)) = (map.remove("token"), map.remove("user"))
        else {
            return Err("the server did not return a session token".to_owned());
        };
        let id = user
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let same = self
            .store
            .meta_get("user_id")
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some(id.as_str());
        if !same {
            self.store.wipe().map_err(|e| e.to_string())?;
            self.store
                .meta_set("user_id", &id)
                .map_err(|e| e.to_string())?;
        }
        self.token.set(&token)?;
        self.paused.store(false, Ordering::Relaxed);
        self.wake.notify_one();
        self.publish();
        Ok(user)
    }

    // --- sign-in with Google (ADR-0062) ----------------------------------------------------------

    /// Starts it: the server is given the SHA-256 of a secret that stays here, and answers with the address
    /// for the system browser. The sign-in ends when a `vumlo-<abbr>://auth/google?code=` link arrives.
    pub async fn google_start(&self, login_hint: Option<String>) -> Result<String, String> {
        let verifier = random_hex(32)?;
        let mut body = json!({"app_challenge": sha256_hex(&verifier)});
        if let Some(hint) = login_hint.filter(|hint| !hint.is_empty()) {
            body["login_hint"] = Value::String(hint);
        }
        let wire = self
            .send("POST", "/auth/google/start", Some(body.to_string()), &[])
            .await
            .inspect_err(|_| self.set_online(false))
            .map_err(|e| e.to_string())?;
        self.set_online(true);
        if wire.status != 200 {
            return Err(format!("Google sign-in is not available ({})", wire.status));
        }
        let url = parse(&wire.text)
            .and_then(|json| {
                json.get("authorization_url")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
            .ok_or("the server did not return an address to sign in at")?;
        *self.google.lock().map_err(|e| e.to_string())? = Some(verifier);
        Ok(url)
    }

    /// Ends it with the code of the link. The secret is used up by the attempt, right or wrong.
    pub async fn google_finish(&self, code: &str) -> Result<Value, String> {
        let verifier = self
            .google
            .lock()
            .map_err(|e| e.to_string())?
            .take()
            .ok_or("no sign-in with Google was started in this app")?;
        let body = json!({"code": code, "verifier": verifier});
        let wire = self
            .send("POST", "/auth/app/exchange", Some(body.to_string()), &[])
            .await
            .inspect_err(|_| self.set_online(false))
            .map_err(|e| e.to_string())?;
        self.set_online(true);
        if wire.status != 200 {
            return Err(parse(&wire.text)
                .and_then(|json| {
                    json.get("detail")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| format!("the sign-in was not accepted ({})", wire.status)));
        }
        self.accept_session(parse(&wire.text))
    }

    async fn logout(&self) -> Reply {
        // Whatever the server says, or whether it can be reached, this device no longer holds a session.
        let _ = self
            .send("POST", "/auth/logout", Some("{}".to_owned()), &[])
            .await;
        self.token.clear();
        let _ = self.store.wipe();
        self.paused.store(false, Ordering::Relaxed);
        self.publish();
        Reply::plain(204, None)
    }

    // --- the queue -------------------------------------------------------------------------------

    /// Sends every write that is due, oldest first, once. Returns how many it tried.
    pub async fn deliver_due(&self) -> usize {
        if self.paused.load(Ordering::Relaxed) || self.token.get().is_none() {
            return 0;
        }
        let mut tried = 0;
        while let Ok(Some(row)) = self.store.next_due(now_ms() / 1000) {
            self.deliver(row).await;
            tried += 1;
            // No connection, or no session: the rest would only fail the same way.
            if self.paused.load(Ordering::Relaxed) || !self.online.load(Ordering::Relaxed) {
                break;
            }
        }
        tried
    }

    async fn deliver(&self, row: OutboxRow) {
        if self.store.mark_sending(row.id).is_err() {
            return;
        }
        let attempts = row.attempts + 1;
        let later = |after: Option<u64>| {
            let wait = backoff_secs(attempts).max(after.unwrap_or(0) as i64);
            let _ = self
                .store
                .mark_retry(row.id, attempts, now_ms() / 1000 + wait);
        };
        match self
            .send(
                &row.method,
                &row.path,
                row.body.clone(),
                &[("idempotency-key", &row.idem_key)],
            )
            .await
        {
            Err(_) => {
                self.set_online(false);
                later(None);
            }
            Ok(wire) => match wire.status {
                200..=299 => {
                    self.set_online(true);
                    let _ = self.store.outbox_delete(row.id);
                    self.sent.fetch_add(1, Ordering::Relaxed);
                }
                401 => {
                    // The session is gone: hold everything, lose nothing, until the person signs in.
                    let _ = self.store.mark_retry(row.id, row.attempts, 0);
                    self.paused.store(true, Ordering::Relaxed);
                }
                status if retryable(status) => {
                    self.set_online(false);
                    later(wire.retry_after);
                }
                status => {
                    self.set_online(true);
                    let _ = self
                        .store
                        .mark_failed(row.id, i64::from(status), Some(&wire.text));
                }
            },
        }
        self.publish();
    }

    /// The loop that sends the queue. It wakes when something is queued, when the connection comes back,
    /// when the person signs in, and when the first back-off ends.
    pub async fn run_outbox(self: Arc<Self>) {
        loop {
            self.deliver_due().await;
            let wait = match self.store.next_wake() {
                Ok(Some(at)) => (at - now_ms() / 1000).clamp(1, 3600) as u64,
                _ => 3600,
            };
            tokio::select! {
                () = self.wake.notified() => {}
                () = tokio::time::sleep(Duration::from_secs(wait)) => {}
            }
        }
    }

    /// "Try now": ask the server whether it is there and send what waits without waiting for a back-off.
    pub async fn sync_now(&self) {
        let up = self.send("GET", "/health", None, &[]).await.is_ok();
        self.set_online(up);
        let _ = self.store.retry_all_now();
        self.wake.notify_one();
        self.publish();
    }

    /// Someone is signed in on this device.
    pub fn signed_in(&self) -> bool {
        self.token.get().is_some()
    }

    /// The language of the window, which the notifications speak (`en` until the window says otherwise).
    pub fn language(&self) -> String {
        self.store
            .meta_get("language")
            .ok()
            .flatten()
            .unwrap_or_else(|| "en".to_owned())
    }

    pub fn set_language(&self, language: &str) -> Result<(), String> {
        self.store
            .meta_set("language", language)
            .map_err(|e| e.to_string())
    }

    /// True the first time this notice is asked about.
    pub fn notice_new(&self, assignment_id: &str, due_at: &str, window: &str) -> bool {
        self.store
            .notified_insert(assignment_id, due_at, window)
            .unwrap_or(false)
    }

    pub fn outbox(&self) -> Result<Vec<OutboxRow>, String> {
        self.store.outbox_list().map_err(|e| e.to_string())
    }

    pub fn discard(&self, id: i64) -> Result<(), String> {
        self.store.outbox_delete(id).map_err(|e| e.to_string())?;
        self.publish();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
