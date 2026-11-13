//! People calling a plugin's own server through the core (ADR-0078). A plugin declares routes in its
//! manifest, each with who may call it. The core signs the person in with the session it already
//! checks (and its CSRF rule), matches the call to a declared route, decides with `access` whether
//! this person may use it, and forwards it with a signed statement of who they are. The plugin never
//! sees a cookie or a session token.
//!
//! What the plugin sends back is served from the core's origin, so it is treated as untrusted
//! content: only a short list of types that cannot run script is passed on, and every answer carries
//! headers that stop a browser from guessing otherwise.

use access::{Actor, ActorSource, PluginRoute, plugin_user};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{FromRef, FromRequestParts, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::any;
use chrono::Utc;
use common::ApiError;
use module_api::OutboundError;
use teaching::{self, TeachingError};
use uuid::Uuid;

use crate::model::{COURSE_SEGMENT, InstallStatus, RouteAuth};
use crate::repo;
use crate::secret::{ProxyCaller, ProxyClaims, derive_secret, proxy_signature};
use crate::state::PluginsState;

/// The most a person may send through the core to a plugin.
pub const MAX_REQUEST_BYTES: usize = 10 * 1024 * 1024;

/// The media types of an answer the core passes on. None of them runs in a browser: no HTML, no
/// SVG, no script. Anything else the plugin sends is a `502`.
const ANSWER_TYPES: [&str; 7] = [
    "application/json",
    "application/problem+json",
    "text/plain",
    "text/csv",
    "text/calendar",
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
];

/// `ANY /plugins/{plugin_id}/{*rest}`, to be nested under `/api/v1`. Body and time limits are the
/// caller's to set: a plugin may take more than the rest of the API does.
pub fn router<S>() -> Router<S>
where
    S: ActorSource + Clone + Send + Sync + 'static,
    PluginsState: FromRef<S>,
{
    Router::new().route("/plugins/{plugin_id}/{*rest}", any(proxy::<S>))
}

fn bad_gateway(plugin: &str, why: &str) -> ApiError {
    tracing::warn!(plugin, why, "a plugin route failed");
    ApiError::Status(StatusCode::BAD_GATEWAY)
}

/// `a/b` of `/plugins/hello/a/b`: the address on the plugin's own server, with its leading `/`.
fn tail_of(path: &str) -> Option<(&str, String)> {
    let (plugin, rest) = path.strip_prefix("/plugins/")?.split_once('/')?;
    Some((plugin, format!("/{rest}")))
}

/// The request-target the plugin receives: the path of its base address, the route, the query.
fn target_of(base_url: &str, rest: &str, query: Option<&str>) -> String {
    let prefix = base_url
        .split_once("://")
        .and_then(|(_, after)| after.find('/').map(|at| &after[at..]))
        .unwrap_or("");
    match query {
        Some(query) => format!("{prefix}{rest}?{query}"),
        None => format!("{prefix}{rest}"),
    }
}

/// The media type of a `Content-Type`, lowercase, without its parameters.
fn media_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

async fn proxy<S>(State(app): State<S>, request: Request) -> Result<Response, ApiError>
where
    S: ActorSource + Send + Sync,
    PluginsState: FromRef<S>,
{
    let state = PluginsState::from_ref(&app);
    let (mut parts, body) = request.into_parts();
    let (plugin_id, rest) = tail_of(parts.uri.path()).ok_or(ApiError::NotFound)?;
    let plugin_id = plugin_id.to_owned();
    // Without the key no plugin is installed.
    let key = state.key.as_deref().ok_or(ApiError::NotFound)?;
    let mut conn = state
        .db
        .acquire()
        .await
        .map_err(|e| ApiError::from_db("plugins", &e))?;
    let install = repo::find_install(&mut conn, &plugin_id)
        .await
        .map_err(|e| ApiError::from_db("plugins", &e))?
        .filter(|install| install.status == InstallStatus::Enabled);
    let verb = parts.method.clone();
    let method = verb.as_str();
    let found = install.as_ref().and_then(|install| {
        install.routes.iter().enumerate().find_map(|(at, route)| {
            (route.method == method)
                .then(|| route.matches(&rest))
                .flatten()
                .map(|params| (at, route, params))
        })
    });
    // A route that is not public needs a person who is signed in, and they are asked for before
    // anything is said about what exists: a stranger gets `401` for a plugin or a route that is not
    // there, as they did before routes could be public.
    let public = found
        .as_ref()
        .is_some_and(|(_, route, _)| route.auth == RouteAuth::None);
    let actor = if public {
        None
    } else {
        Some(Actor::from_request_parts(&mut parts, &app).await?)
    };
    let (install, (route_at, route, params)) =
        install.as_ref().zip(found).ok_or(ApiError::NotFound)?;

    // Who calls. A person: the course in the path is one they can see (else it is not there), then
    // one the plugin may reach, then the rules of the route. Anyone: only the allowance of the route.
    let (caller, course, course_role) = match &actor {
        Some(actor) => {
            let course = params
                .iter()
                .find(|(name, _)| *name == COURSE_SEGMENT)
                .map(|(_, value)| value.parse::<Uuid>().map_err(|_| ApiError::NotFound))
                .transpose()?;
            let access = match course {
                Some(course_id) => {
                    let access = teaching::course_access(&mut conn, actor, course_id)
                        .await
                        .map_err(|error| match error {
                            TeachingError::CourseNotFound => ApiError::NotFound,
                            other => other.into(),
                        })?;
                    if !install.courses.allows(course_id) {
                        return Err(ApiError::Forbidden("the plugin does not reach this course"));
                    }
                    Some(access)
                }
                None => None,
            };
            access::authorize_plugin_route(
                actor,
                PluginRoute {
                    roles: &route.roles,
                    course_roles: &route.course_roles,
                },
                access,
            )?;
            let course_role = access.and_then(|a| a.role).map(|role| role.as_str());
            (Some(plugin_user(actor)), course, course_role)
        }
        None => {
            state
                .public_routes
                .check((install.id, route_at))
                .map_err(|wait| ApiError::TooManyRequests {
                    retry_after_secs: Some(wait.as_secs() + u64::from(wait.subsec_nanos() > 0)),
                })?;
            (None, None, None)
        }
    };
    let secret =
        derive_secret(key, install.id, install.secret_version).map_err(|_| ApiError::Internal)?;
    let client = state.proxy.as_deref().ok_or(ApiError::ServiceUnavailable(
        "plugin routes are not available",
    ))?;
    // Nothing below needs the connection, and the plugin may take a while.
    drop(conn);

    let body = to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| ApiError::Status(StatusCode::PAYLOAD_TOO_LARGE))?;
    let query = parts.uri.query();
    let target = target_of(&install.base_url, &rest, query);
    let claims = ProxyClaims {
        method,
        target: &target,
        caller: caller.as_ref().map(|user| ProxyCaller {
            user: user.id,
            role: user.role.as_str(),
        }),
        course,
        course_role,
        nonce: Uuid::now_v7(),
        timestamp: Utc::now().timestamp(),
    };
    let signature = proxy_signature(&secret, &claims, &body).map_err(|_| ApiError::Internal)?;
    let timestamp = claims.timestamp.to_string();
    let nonce = claims.nonce.to_string();
    let course_text = course.map(|c| c.to_string());
    // Only these are passed on from the caller: never a cookie, never an authorization header, and
    // nothing named `X-Vumlo-*` (the plugin trusts those, and only from the lines below).
    let mut headers: Vec<(&str, &str)> = Vec::new();
    for name in ["content-type", "accept", "accept-language"] {
        if let Some(value) = parts.headers.get(name).and_then(|v| v.to_str().ok()) {
            headers.push((name, value));
        }
    }
    headers.extend([
        ("x-vumlo-timestamp", timestamp.as_str()),
        ("x-vumlo-nonce", nonce.as_str()),
        ("x-vumlo-signature", signature.as_str()),
    ]);
    let user_id = caller.map(|user| user.id.to_string());
    match (&caller, &user_id) {
        (Some(user), Some(id)) => {
            headers.push(("x-vumlo-user", id));
            headers.push(("x-vumlo-role", user.role.as_str()));
        }
        // The signature says it too (its `none` line): this header is for the plugin's convenience.
        _ => headers.push(("x-vumlo-anonymous", "1")),
    }
    if let Some(course) = &course_text {
        headers.push(("x-vumlo-course", course));
    }
    if let Some(role) = course_role {
        headers.push(("x-vumlo-course-role", role));
    }
    let url = format!(
        "{}{rest}{}",
        install.base_url,
        query.map_or(String::new(), |q| format!("?{q}"))
    );
    let reply = client
        .request(method, &url, &headers, body.to_vec())
        .await
        .map_err(|error| match error {
            OutboundError::Timeout => {
                tracing::warn!(plugin = plugin_id.as_str(), "a plugin route timed out");
                ApiError::Status(StatusCode::GATEWAY_TIMEOUT)
            }
            other => bad_gateway(&plugin_id, &other.to_string()),
        })?;

    // 2xx and 4xx are the plugin's answer to the person. Anything else is the plugin failing.
    let status =
        StatusCode::from_u16(reply.status).map_err(|_| bad_gateway(&plugin_id, "status"))?;
    if !(status.is_success() || status.is_client_error()) {
        return Err(bad_gateway(
            &plugin_id,
            "the plugin answered neither 2xx nor 4xx",
        ));
    }
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    // A 4xx is the plugin's to say, in its own words.
    response.extensions_mut().insert(common::PassThrough);
    let carries_body = !reply.body.is_empty();
    if carries_body {
        let content_type = reply
            .content_type
            .as_deref()
            .ok_or_else(|| bad_gateway(&plugin_id, "no content type"))?;
        if !ANSWER_TYPES.contains(&media_type(content_type).as_str()) {
            return Err(bad_gateway(
                &plugin_id,
                "a content type the core does not pass on",
            ));
        }
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(content_type)
                .map_err(|_| bad_gateway(&plugin_id, "content type"))?,
        );
        // A download keeps its name; anything that is not a download is not passed on.
        if let Some(disposition) = reply
            .content_disposition
            .as_deref()
            .filter(|d| {
                d.trim_start()
                    .to_ascii_lowercase()
                    .starts_with("attachment")
            })
            .and_then(|d| HeaderValue::from_str(d).ok())
        {
            response
                .headers_mut()
                .insert(header::CONTENT_DISPOSITION, disposition);
        }
        *response.body_mut() = Body::from(reply.body);
    }
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'"),
    );
    Ok(response)
}

mod docs;
pub use docs::openapi;

#[cfg(test)]
mod tests;
