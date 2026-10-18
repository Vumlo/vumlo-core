//! `GET /admin/audit` (ADR-0053).

use api_types::{
    AuditActor, AuditKind, AuditQuery, AuditRecord, AuditSource, Page, ProblemDetails,
};
use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::model::{AuditFilter, AuditRow};
use super::service::{self, EventsError};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidQuery};
use crate::pagination::PageParams;

const DEFAULT_DAYS: u32 = 7;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_audit))
}

fn kind_name(kind: AuditKind) -> &'static str {
    match kind {
        AuditKind::Accounts => "accounts",
        AuditKind::Owners => "owners",
        AuditKind::Teachers => "teachers",
        AuditKind::Groups => "groups",
        AuditKind::Courses => "courses",
        AuditKind::Content => "content",
        AuditKind::Grades => "grades",
        AuditKind::Terms => "terms",
        AuditKind::Other => "other",
    }
}

fn api_kind(name: &str) -> AuditKind {
    match name {
        "accounts" => AuditKind::Accounts,
        "owners" => AuditKind::Owners,
        "teachers" => AuditKind::Teachers,
        "groups" => AuditKind::Groups,
        "courses" => AuditKind::Courses,
        "content" => AuditKind::Content,
        "grades" => AuditKind::Grades,
        "terms" => AuditKind::Terms,
        _ => AuditKind::Other,
    }
}

fn api_record(row: AuditRow) -> AuditRecord {
    AuditRecord {
        id: row.id,
        at: row.at,
        source: if row.source == "cli" {
            AuditSource::Cli
        } else {
            AuditSource::Api
        },
        kind: api_kind(&row.kind),
        action: row.action,
        actor: row.actor.map(|(user_id, name, surname)| AuditActor {
            user_id,
            name,
            surname,
        }),
        resource_type: row.resource_type,
        resource_id: row.resource_id,
        resource_label: row.resource_label,
        meta: row.meta,
    }
}

/// The audit log, newest first and paged: who did what to what, when. `kind` is derived from the
/// action; `q` matches the actor, the action or the target; `days` is how far back (1 to 365,
/// default 7). Admin only. The names are those of today.
#[utoipa::path(
    get,
    path = "/admin/audit",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    params(AuditQuery),
    responses(
        (status = 200, description = "A page of entries", body = Page<AuditRecord>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid filter or cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_audit(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<AuditQuery>,
) -> Result<Json<Page<AuditRecord>>, ApiError> {
    let page = PageParams::new(query.cursor.as_deref(), query.limit)?;
    let filter = AuditFilter {
        text: query.q.filter(|q| !q.trim().is_empty()),
        kind: query.kind.map(|kind| kind_name(kind).to_owned()),
        days: i32::try_from(query.days.unwrap_or(DEFAULT_DAYS)).unwrap_or(365),
    };
    let mut conn = state.db.acquire().await.map_err(EventsError::from)?;
    let rows = service::list_audit(
        &mut conn,
        &actor,
        filter,
        page.after,
        i64::from(page.limit) + 1,
    )
    .await?;
    let page = crate::pagination::Page::from_rows(rows, page.limit, |row| row.id);
    Ok(Json(Page {
        items: page.items.into_iter().map(api_record).collect(),
        next_cursor: page.next.map(|id| id.to_string()),
    }))
}
