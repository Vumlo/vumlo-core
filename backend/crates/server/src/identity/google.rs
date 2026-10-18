//! The outbound half of sign-in with Google (ADR-0047): discovery, the authorization URL and the
//! exchange of the code. It touches no database; `service` stores the state and resolves the account.

use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::reqwest;
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use tokio::sync::OnceCell;

use vumlo_config::GoogleSettings;

/// The configuration cannot be applied. The message names the key and the value.
#[derive(Debug, thiserror::Error)]
pub enum GoogleSetupError {
    #[error("auth.google.issuer \"{0}\" is not a URL")]
    Issuer(String),
    #[error("auth.google.redirect_uri \"{0}\" is not a URL")]
    RedirectUri(String),
    #[error("cannot build the HTTP client for Google: {0}")]
    Client(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GoogleError {
    /// Google could not be reached or its discovery document is unusable. Worth trying again.
    #[error("Google is not available")]
    Unavailable,
    /// Google refused the code, or the ID token does not verify.
    #[error("Google did not accept the sign-in")]
    Rejected,
}

/// What `start` hands to the browser, and what `callback` needs to remember about it.
pub struct Begin {
    pub authorization_url: String,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

/// The verified claims `callback` needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleProfile {
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}

pub struct Google {
    client_id: ClientId,
    client_secret: ClientSecret,
    issuer: IssuerUrl,
    redirect: RedirectUrl,
    http: reqwest::Client,
    /// Fetched on the first use and kept; a failed fetch is tried again by the next request.
    metadata: OnceCell<CoreProviderMetadata>,
}

impl Google {
    pub fn new(settings: &GoogleSettings) -> Result<Self, GoogleSetupError> {
        let http = reqwest::ClientBuilder::new()
            // Following redirects would let a hostile discovery document point us at internal hosts.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| GoogleSetupError::Client(e.to_string()))?;
        Ok(Self {
            client_id: ClientId::new(settings.client_id.clone()),
            client_secret: ClientSecret::new(settings.client_secret.expose().to_owned()),
            issuer: IssuerUrl::new(settings.issuer.clone())
                .map_err(|_| GoogleSetupError::Issuer(settings.issuer.clone()))?,
            redirect: RedirectUrl::new(settings.redirect_uri.clone())
                .map_err(|_| GoogleSetupError::RedirectUri(settings.redirect_uri.clone()))?,
            http,
            metadata: OnceCell::new(),
        })
    }

    async fn metadata(&self) -> Result<&CoreProviderMetadata, GoogleError> {
        self.metadata
            .get_or_try_init(|| async {
                CoreProviderMetadata::discover_async(self.issuer.clone(), &self.http)
                    .await
                    .map_err(|error| {
                        tracing::warn!(%error, "Google discovery failed");
                        GoogleError::Unavailable
                    })
            })
            .await
    }

    /// The URL to send the browser to, with a fresh state, nonce and PKCE challenge.
    pub async fn begin(&self, login_hint: Option<&str>) -> Result<Begin, GoogleError> {
        let client = CoreClient::from_provider_metadata(
            self.metadata().await?.clone(),
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_redirect_uri(self.redirect.clone());
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut request = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("email".to_owned()))
            .add_scope(Scope::new("profile".to_owned()))
            .set_pkce_challenge(challenge);
        if let Some(hint) = login_hint.filter(|hint| !hint.is_empty()) {
            request = request.add_extra_param("login_hint", hint);
        }
        let (url, state, nonce) = request.url();
        Ok(Begin {
            authorization_url: url.to_string(),
            state: state.secret().clone(),
            nonce: nonce.secret().clone(),
            pkce_verifier: verifier.secret().clone(),
        })
    }

    /// Exchanges the code and verifies the ID token (signature, issuer, audience, expiry and the
    /// nonce of this attempt).
    pub async fn finish(
        &self,
        code: String,
        nonce: String,
        pkce_verifier: String,
    ) -> Result<GoogleProfile, GoogleError> {
        let client = CoreClient::from_provider_metadata(
            self.metadata().await?.clone(),
            self.client_id.clone(),
            Some(self.client_secret.clone()),
        )
        .set_redirect_uri(self.redirect.clone());
        let response = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|_| GoogleError::Unavailable)?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
            .request_async(&self.http)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Google refused the code");
                GoogleError::Rejected
            })?;
        let id_token = response.id_token().ok_or(GoogleError::Rejected)?;
        let claims = id_token
            .claims(&client.id_token_verifier(), &Nonce::new(nonce))
            .map_err(|error| {
                tracing::warn!(%error, "the ID token from Google does not verify");
                GoogleError::Rejected
            })?;
        Ok(GoogleProfile {
            subject: claims.subject().as_str().to_owned(),
            email: claims
                .email()
                .map(|email| email.as_str().to_owned())
                .ok_or(GoogleError::Rejected)?,
            email_verified: claims.email_verified().unwrap_or(false),
            given_name: claims
                .given_name()
                .and_then(|name| name.get(None))
                .map(|name| name.as_str().to_owned()),
            family_name: claims
                .family_name()
                .and_then(|name| name.get(None))
                .map(|name| name.as_str().to_owned()),
        })
    }
}
