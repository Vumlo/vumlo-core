//! Mail from a plugin (ADR-0085): `POST /plugin-api/mail`. The plugin names people by id and writes
//! plain text; the core finds the addresses, queues one message per person and sends them later.
//! The plugin never sees an address.
//!
//! The answers come in this order: `401` for the token, `422` for the body, `403` for the scope,
//! `503` without `[mail]`, `404`/`403` for the target, `429` for a limit.

use access::{PluginCaller, PluginSource, Scope, authorize_plugin};
use api_types::{
    MailAudience, MailTarget, ProblemDetails, SendMailRequest, SendMailResponse, SkippedReasons,
};
use axum::Json;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use common::{ApiError, ValidJson};
use learning::plugin_read;
use mail::{Address, Body, DedupeKey, Kind, LinkPath, PluginMail, Recipient, Subject};
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::db;
use crate::service;
use crate::state::PluginsState;

pub fn router<S>() -> OpenApiRouter<S>
where
    S: PluginSource + Clone + 'static,
    PluginsState: FromRef<S>,
{
    OpenApiRouter::new().routes(routes!(send_mail))
}

fn audience_of(audience: MailAudience) -> plugin_read::MailAudience {
    match audience {
        MailAudience::Students => plugin_read::MailAudience::Students,
        MailAudience::Teachers => plugin_read::MailAudience::Teachers,
        MailAudience::All => plugin_read::MailAudience::All,
    }
}

/// The target as the audit row records it: ids only.
fn target_meta(target: MailTarget) -> Value {
    match target {
        MailTarget::User { user_id } => json!({ "user_id": user_id }),
        MailTarget::Course {
            course_id,
            audience,
        } => json!({
            "course_id": course_id,
            "audience": match audience {
                MailAudience::Students => "students",
                MailAudience::Teachers => "teachers",
                MailAudience::All => "all",
            },
        }),
    }
}

/// Sends plain-text mail in the institution's name to one person or to a course audience. The
/// scope `mail:send`. The mail comes from `mail.from`, ends with a footer that names the plugin and
/// gives a one-click opt-out per `kind`, and is sent within seconds by the mail worker, so `202`
/// means queued, not delivered. A person who opted out, has no verified address, has a disabled
/// account, already got this `dedupe_key`, or already got `mail.recipient_daily` plugin messages in
/// 24 hours is skipped and counted in `skipped_reasons`.
///
/// Limits: `mail.plugin_hourly_calls` accepted calls per plugin per hour and `mail.recipient_daily`
/// messages per person per day are `429` with `Retry-After` (for an audience the daily limit skips
/// the person instead); an audience of more than `mail.max_audience` people is `422` on `to` and
/// queues nothing.
#[utoipa::path(
    post,
    path = "/plugin-api/mail",
    tag = "plugin-api",
    security(("plugin" = [])),
    request_body = SendMailRequest,
    responses(
        (status = 202, description = "Queued", body = SendMailResponse),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `mail:send`, or the person or course is outside the courses it reaches",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such person or course", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A field is not valid, or the audience is larger than `mail.max_audience`",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "The plugin made too many calls this hour, or the person got too much mail today. `Retry-After` says when to try again.",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`[mail]` is not configured on this server", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn send_mail(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    ValidJson(body): ValidJson<SendMailRequest>,
) -> Result<(StatusCode, Json<SendMailResponse>), ApiError> {
    let kind = Kind::parse(&body.kind).map_err(mail::MailError::from)?;
    let subject = Subject::parse(&body.subject).map_err(mail::MailError::from)?;
    let text = Body::parse(&body.text).map_err(mail::MailError::from)?;
    let link_path = body
        .link_path
        .as_deref()
        .map(LinkPath::parse)
        .transpose()
        .map_err(mail::MailError::from)?;
    let dedupe_key = body
        .dedupe_key
        .as_deref()
        .map(DedupeKey::parse)
        .transpose()
        .map_err(mail::MailError::from)?;
    authorize_plugin(&caller, Scope::MailSend)?;
    let (Some(limits), Some(max_audience)) = (state.mail.limits(), state.mail.max_audience())
    else {
        return Err(ApiError::ServiceUnavailable("mail is not configured"));
    };

    // The reads first, in no transaction (CLAUDE.md section 4).
    let target = match body.to {
        MailTarget::User { user_id } => plugin_read::MailTarget::User(user_id),
        MailTarget::Course {
            course_id,
            audience,
        } => plugin_read::MailTarget::Course {
            course_id,
            audience: audience_of(audience),
        },
    };
    let found = {
        let mut conn = state.db.acquire().await.map_err(db)?;
        plugin_read::mail_recipients(&mut conn, &caller, target, max_audience).await?
    };
    let mut no_address = found.no_address;
    let recipients: Vec<Recipient> = found
        .people
        .into_iter()
        .filter_map(|person| match Address::parse(&person.email) {
            Ok(address) => Some(Recipient {
                user_id: person.user_id,
                name: person.name,
                address,
            }),
            Err(_) => {
                no_address += 1;
                None
            }
        })
        .collect();

    let message = PluginMail {
        // The service puts the caller's id here.
        plugin_id: String::new(),
        kind,
        subject,
        body: text,
        link_path,
        dedupe_key,
        recipients,
        single: matches!(body.to, MailTarget::User { .. }),
        no_address,
        disabled: found.disabled,
    };
    let mut tx = state.db.begin().await.map_err(db)?;
    let outcome =
        service::send_mail(&mut tx, &caller, &limits, message, target_meta(body.to)).await?;
    tx.commit().await.map_err(db)?;
    let skipped = outcome.skipped;
    Ok((
        StatusCode::ACCEPTED,
        Json(SendMailResponse {
            accepted: outcome.accepted,
            skipped: skipped.total(),
            skipped_reasons: SkippedReasons {
                opted_out: skipped.opted_out,
                no_address: skipped.no_address,
                duplicate: skipped.duplicate,
                disabled: skipped.disabled,
                rate_limited: skipped.rate_limited,
            },
        }),
    ))
}
