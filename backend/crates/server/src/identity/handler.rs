//! The outbox handlers of `identity` (ADR-0045, ADR-0046): mint the token in a short transaction,
//! then send the mail, with no transaction open.

use std::sync::Arc;

use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::model::TokenPurpose;
use super::service;
use crate::events::{HandlerError, HandlerFuture, Handlers, OutboxEvent};
use crate::mail::{MailError, Mailer};
use vumlo_config::Language;

/// What the handlers need besides the pool.
#[derive(Clone)]
pub struct MailContext {
    pub mailer: Arc<Mailer>,
    pub language: Language,
    pub institution_name: String,
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
    let minted = {
        let mut tx = pool.begin().await.map_err(database)?;
        let minted = service::mint_email_token(&mut tx, payload.user_id, purpose)
            .await
            .map_err(|_| HandlerError("cannot mint the token".to_owned()))?;
        tx.commit().await.map_err(database)?;
        minted
    };
    let Some(minted) = minted else {
        return Ok(());
    };
    let path = match purpose {
        TokenPurpose::Verify => "/verify-email",
        TokenPurpose::Reset => "/reset-password",
    };
    let link = context
        .mailer
        .link(&format!("{path}?token={}", minted.token.expose()));
    let (subject, body) = text(
        purpose,
        context.language,
        &minted.name,
        &context.institution_name,
        &link,
    );
    context
        .mailer
        .send(
            &format!("{} {}", minted.name, minted.surname),
            &minted.email,
            subject,
            body,
        )
        .await
        .map_err(|error| {
            // The transport's own text can name the address, so only the kind of failure is kept.
            HandlerError(
                match error {
                    MailError::Send(_) => "the mail server did not accept the message",
                    _ => "the message could not be built",
                }
                .to_owned(),
            )
        })
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
                "Hello {name},\n\nTo finish creating your {institution} account, confirm your email address:\n\n{link}\n\nThe link works once and expires in 24 hours. If you did not sign up, ignore this message.\n"
            ),
        ),
        (TokenPurpose::Verify, Language::Uk) => (
            "Підтвердіть адресу електронної пошти",
            format!(
                "Вітаємо, {name}!\n\nЩоб завершити створення облікового запису в «{institution}», підтвердьте адресу електронної пошти:\n\n{link}\n\nПосилання діє один раз і спливає за 24 години. Якщо ви не реєструвалися, проігноруйте цей лист.\n"
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
