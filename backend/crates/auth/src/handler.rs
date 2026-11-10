//! The outbox handlers of `auth` (ADR-0045, ADR-0046, ADR-0084): mint the token and queue the mail in
//! one short transaction. The mail layer sends it later; nothing here touches a transport.

use mail::{AccountMail, Address, Body, Kind, Subject};
use sqlx::PgPool;
use uuid::Uuid;

use super::model::TokenPurpose;
use super::service;
use events::{HandlerError, HandlerFuture, Handlers, OutboxEvent};
use serde::Deserialize;
use vumlo_config::{Language, Settings};

/// What the handlers need besides the pool.
#[derive(Clone)]
pub struct MailContext {
    pub language: Language,
    pub institution_name: String,
    /// Where the links in the mails point: `mail.link_base`, else `server.public_origin`.
    pub link_base: String,
}

impl MailContext {
    pub fn new(config: &Settings) -> Self {
        let link_base = config
            .mail
            .as_ref()
            .and_then(|mail| mail.link_base.clone())
            .unwrap_or_else(|| config.server.public_origin.clone());
        Self {
            language: config.institution.language,
            institution_name: config.institution.name.clone(),
            link_base,
        }
    }
}

pub fn event_handlers(pool: PgPool, context: MailContext) -> Handlers {
    let handler = |purpose: TokenPurpose| {
        let (pool, context) = (pool.clone(), context.clone());
        move |event: OutboxEvent| -> HandlerFuture {
            let (pool, context) = (pool.clone(), context.clone());
            Box::pin(async move { deliver(&pool, &context, purpose, &event).await })
        }
    };
    Handlers::default()
        .on("user.verification_requested", handler(TokenPurpose::Verify))
        .on(
            "user.password_reset_requested",
            handler(TokenPurpose::Reset),
        )
}

#[derive(Deserialize)]
struct Payload {
    user_id: Uuid,
}

/// Errors here are stored in the outbox row, so they never carry an address or a name.
async fn deliver(
    pool: &PgPool,
    context: &MailContext,
    purpose: TokenPurpose,
    event: &OutboxEvent,
) -> Result<(), HandlerError> {
    let payload: Payload = serde_json::from_value(event.payload.clone())
        .map_err(|_| HandlerError("the payload has no user_id".to_owned()))?;
    let database = |_| HandlerError("the database is not available".to_owned());
    let mut tx = pool.begin().await.map_err(database)?;
    // A rerun of the event must not mint a second token: the first mail is queued already.
    if mail::origin_queued(&mut tx, event.id)
        .await
        .map_err(|_| HandlerError("cannot read the mail queue".to_owned()))?
    {
        return Ok(());
    }
    let Some(minted) = service::mint_email_token(&mut tx, payload.user_id, purpose)
        .await
        .map_err(|_| HandlerError("cannot mint the token".to_owned()))?
    else {
        return Ok(());
    };
    let path = match purpose {
        TokenPurpose::Verify => "/verify-email",
        TokenPurpose::Reset => "/reset-password",
    };
    let link = format!("{}{path}?token={}", context.link_base, minted.token.expose());
    let (subject, body) = text(
        purpose,
        context.language,
        &minted.name,
        &context.institution_name,
        &link,
    );
    let unfit = |_| HandlerError("the message does not fit the mail queue".to_owned());
    let kind = match purpose {
        TokenPurpose::Verify => "verify-email",
        TokenPurpose::Reset => "password-reset",
    };
    mail::enqueue_account(
        &mut tx,
        AccountMail {
            origin_id: event.id,
            kind: Kind::parse(kind).map_err(unfit)?,
            user_id: payload.user_id,
            name: format!("{} {}", minted.name, minted.surname),
            address: Address::parse(&minted.email).map_err(unfit)?,
            subject: Subject::parse(subject).map_err(unfit)?,
            body: Body::parse(&body).map_err(unfit)?,
        },
    )
    .await
    .map_err(|_| HandlerError("cannot queue the message".to_owned()))?;
    tx.commit().await.map_err(database)?;
    Ok(())
}

/// The subject and the plain-text body of a mail, in the institution's language.
fn text(
    purpose: TokenPurpose,
    language: Language,
    name: &str,
    institution: &str,
    link: &str,
) -> (&'static str, String) {
    match (purpose, language) {
        (TokenPurpose::Verify, Language::En) => (
            "Confirm your email address",
            format!(
                "Hello {name},\n\nTo finish creating your {institution} account, confirm your email address and choose a password:\n\n{link}\n\nThe link works once and expires in 24 hours. If you did not sign up, ignore this message.\n"
            ),
        ),
        (TokenPurpose::Verify, Language::Uk) => (
            "Підтвердіть адресу електронної пошти",
            format!(
                "Вітаємо, {name}!\n\nЩоб завершити створення облікового запису в «{institution}», підтвердьте адресу електронної пошти та оберіть пароль:\n\n{link}\n\nПосилання діє один раз і спливає за 24 години. Якщо ви не реєструвалися, проігноруйте цей лист.\n"
            ),
        ),
        (TokenPurpose::Reset, Language::En) => (
            "Reset your password",
            format!(
                "Hello {name},\n\nSomeone asked to reset the password of your {institution} account. To choose a new password, open:\n\n{link}\n\nThe link works once and expires in 1 hour. If this was not you, ignore this message; your password stays as it is.\n"
            ),
        ),
        (TokenPurpose::Reset, Language::Uk) => (
            "Скидання пароля",
            format!(
                "Вітаємо, {name}!\n\nХтось попросив скинути пароль вашого облікового запису в «{institution}». Щоб вибрати новий пароль, відкрийте:\n\n{link}\n\nПосилання діє один раз і спливає за 1 годину. Якщо це були не ви, проігноруйте цей лист — пароль залишиться без змін.\n"
            ),
        ),
    }
}

#[cfg(test)]
mod tests;
