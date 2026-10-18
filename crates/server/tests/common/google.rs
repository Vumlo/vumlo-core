//! A fake OpenID Connect issuer for the tests of sign-in with Google (ADR-0047): discovery, an
//! empty key set and a token endpoint that checks the PKCE verifier. It advertises HS256 and signs
//! ID tokens with the client secret, which the library accepts, so no RSA key is needed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

pub const CLIENT_ID: &str = "learnly-test-client";
pub const CLIENT_SECRET: &str = "a client secret that is long enough for HS256";
pub const REDIRECT_URI: &str = "https://lms.test/auth/google/callback";

/// Who Google says the person is.
#[derive(Clone)]
pub struct Person {
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    /// Put another nonce in the ID token than the one of the request.
    pub wrong_nonce: bool,
}

impl Person {
    pub fn new(subject: &str, email: &str) -> Self {
        Self {
            subject: subject.to_owned(),
            email: email.to_owned(),
            email_verified: true,
            given_name: Some("Maya".to_owned()),
            family_name: Some("Okafor".to_owned()),
            wrong_nonce: false,
        }
    }
}

struct Grant {
    person: Person,
    nonce: String,
    challenge: String,
}

#[derive(Clone)]
struct Issuer {
    base: String,
    grants: Arc<Mutex<HashMap<String, Grant>>>,
}

pub struct FakeIssuer {
    pub base: String,
    grants: Arc<Mutex<HashMap<String, Grant>>>,
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key = if key.len() > 64 {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    key.resize(64, 0);
    let inner: Vec<u8> = key
        .iter()
        .map(|b| b ^ 0x36)
        .chain(message.iter().copied())
        .collect();
    let outer: Vec<u8> = key
        .iter()
        .map(|b| b ^ 0x5c)
        .chain(Sha256::digest(inner))
        .collect();
    Sha256::digest(outer).into()
}

fn id_token(base: &str, grant: &Grant) -> String {
    let encode = |value: &Value| URL_SAFE_NO_PAD.encode(value.to_string());
    let now = chrono::Utc::now().timestamp();
    let person = &grant.person;
    let mut claims = json!({
        "iss": base,
        "sub": person.subject,
        "aud": CLIENT_ID,
        "iat": now,
        "exp": now + 300,
        "nonce": if person.wrong_nonce { "another nonce" } else { grant.nonce.as_str() },
        "email": person.email,
        "email_verified": person.email_verified,
    });
    if let Some(name) = &person.given_name {
        claims["given_name"] = name.clone().into();
    }
    if let Some(name) = &person.family_name {
        claims["family_name"] = name.clone().into();
    }
    let signing_input = format!(
        "{}.{}",
        encode(&json!({"alg": "HS256", "typ": "JWT"})),
        encode(&claims)
    );
    let signature = URL_SAFE_NO_PAD.encode(hmac_sha256(
        CLIENT_SECRET.as_bytes(),
        signing_input.as_bytes(),
    ));
    format!("{signing_input}.{signature}")
}

async fn discovery(State(issuer): State<Issuer>) -> Json<Value> {
    let base = &issuer.base;
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "jwks_uri": format!("{base}/jwks"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["HS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
    }))
}

async fn jwks() -> Json<Value> {
    Json(json!({"keys": []}))
}

async fn token(
    State(issuer): State<Issuer>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let refuse = |error: &str| (StatusCode::BAD_REQUEST, Json(json!({"error": error})));
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return Err(refuse("unsupported_grant_type"));
    }
    if form.get("redirect_uri").map(String::as_str) != Some(REDIRECT_URI) {
        return Err(refuse("invalid_grant"));
    }
    let grant = {
        let code = form.get("code").ok_or_else(|| refuse("invalid_request"))?;
        issuer
            .grants
            .lock()
            .unwrap()
            .remove(code)
            .ok_or_else(|| refuse("invalid_grant"))?
    };
    let verifier = form
        .get("code_verifier")
        .ok_or_else(|| refuse("invalid_grant"))?;
    if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != grant.challenge {
        return Err(refuse("invalid_grant"));
    }
    Ok(Json(json!({
        "access_token": "an-access-token",
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": id_token(&issuer.base, &grant),
    })))
}

impl FakeIssuer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind issuer");
        let base = format!("http://{}", listener.local_addr().expect("issuer address"));
        let grants = Arc::new(Mutex::new(HashMap::new()));
        let issuer = Issuer {
            base: base.clone(),
            grants: grants.clone(),
        };
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/token", post(token))
            .with_state(issuer);
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve issuer");
        });
        Self { base, grants }
    }

    /// What Google does after the person agrees: checks the authorization request the server made,
    /// remembers who signed in and returns the `(code, state)` of the redirect to the web app.
    pub fn approve(&self, authorization_url: &str, person: Person) -> (String, String) {
        let url = openidconnect::url::Url::parse(authorization_url).expect("an authorization URL");
        assert_eq!(url.path(), "/authorize", "{authorization_url}");
        let query: HashMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["client_id"], CLIENT_ID);
        assert_eq!(query["redirect_uri"], REDIRECT_URI);
        assert_eq!(query["code_challenge_method"], "S256");
        for scope in ["openid", "email", "profile"] {
            assert!(
                query["scope"].split(' ').any(|s| s == scope),
                "{}",
                query["scope"]
            );
        }
        let code = format!("code-{}", uuid::Uuid::now_v7());
        self.grants.lock().unwrap().insert(
            code.clone(),
            Grant {
                person,
                nonce: query["nonce"].clone(),
                challenge: query["code_challenge"].clone(),
            },
        );
        (code, query["state"].clone())
    }
}
