use api_types::{FieldError, ProblemDetails};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use garde::{Report, Validate};
use serde::de::DeserializeOwned;

const PROBLEM_JSON: &str = "application/problem+json";
/// What a `503` tells a client to wait.
const RETRY_AFTER_UNAVAILABLE_SECS: u64 = 5;

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    /// Authentication is missing or wrong. The detail must not reveal which part failed.
    Unauthorized(&'static str),
    /// The caller is known but may not do this.
    Forbidden(&'static str),
    Conflict(&'static str),
    /// The resource existed once and is gone for good, e.g. a spent or expired link.
    Gone(&'static str),
    Unprocessable {
        detail: String,
        errors: Vec<FieldError>,
    },
    TooManyRequests {
        retry_after_secs: Option<u64>,
    },
    ServiceUnavailable(&'static str),
    /// The cause has been logged where the error was created; the response stays generic.
    Internal,
    /// A status produced outside our handlers (405, 408, 413, ...).
    Status(StatusCode),
}

impl ApiError {
    /// A `422` that blames one field.
    pub fn invalid_field(field: &str, message: impl Into<String>) -> Self {
        Self::Unprocessable {
            detail: "the request has invalid fields".to_owned(),
            errors: vec![FieldError {
                field: field.to_owned(),
                message: message.into(),
            }],
        }
    }

    /// The response for a database error met in `module`. A database that cannot be reached, a pool
    /// that has no connection to give within its timeout and a server that is full or shutting down
    /// are `503` with `Retry-After`: the request can be tried again. Anything else is logged and
    /// answered with a generic `500` (ADR-0039).
    pub fn from_db(module: &str, error: &sqlx::Error) -> Self {
        if is_unavailable(error) {
            tracing::warn!(error = %error, "{module}: the database is unavailable");
            return Self::ServiceUnavailable("the database is busy or unavailable, try again");
        }
        tracing::error!(error = %error, "{module}: database error");
        Self::Internal
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Gone(_) => StatusCode::GONE,
            Self::Unprocessable { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Status(status) => *status,
        }
    }
}

/// Errors after which the same request may well succeed.
fn is_unavailable(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => true,
        sqlx::Error::WorkerCrashed => true,
        sqlx::Error::Database(db) => db.code().is_some_and(|code| {
            // 08: connection exception; 53: insufficient resources (too many connections, disk,
            // memory); 57P: operator intervention (shutdown, restart).
            code.starts_with("08") || code.starts_with("53") || code.starts_with("57P")
        }),
        _ => false,
    }
}

impl From<crate::policy::PolicyError> for ApiError {
    fn from(error: crate::policy::PolicyError) -> Self {
        match error {
            crate::policy::PolicyError::Denied => Self::Forbidden("not allowed"),
            crate::policy::PolicyError::Archived => Self::Conflict("the course is archived"),
        }
    }
}

impl From<crate::events::EventsError> for ApiError {
    fn from(error: crate::events::EventsError) -> Self {
        match error {
            crate::events::EventsError::Forbidden(policy) => policy.into(),
            crate::events::EventsError::Db(e) => Self::from_db("events", &e),
        }
    }
}

impl From<Report> for ApiError {
    fn from(report: Report) -> Self {
        Self::Unprocessable {
            detail: "the request has invalid fields".to_owned(),
            errors: report
                .iter()
                .map(|(path, error)| FieldError {
                    field: path.to_string(),
                    message: error.message().to_owned(),
                })
                .collect(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let mut extra_headers = Vec::new();
        let (detail, errors) = match self {
            Self::Unauthorized(detail) => {
                extra_headers.push((header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer")));
                (Some(detail.to_owned()), Vec::new())
            }
            Self::Forbidden(detail) | Self::Conflict(detail) | Self::Gone(detail) => {
                (Some(detail.to_owned()), Vec::new())
            }
            Self::ServiceUnavailable(detail) => {
                extra_headers.push((
                    header::RETRY_AFTER,
                    HeaderValue::from(RETRY_AFTER_UNAVAILABLE_SECS),
                ));
                (Some(detail.to_owned()), Vec::new())
            }
            Self::Unprocessable { detail, errors } => (Some(detail), errors),
            Self::TooManyRequests { retry_after_secs } => {
                if let Some(secs) = retry_after_secs {
                    extra_headers.push((header::RETRY_AFTER, HeaderValue::from(secs)));
                }
                (
                    Some("too many attempts, try again later".to_owned()),
                    Vec::new(),
                )
            }
            Self::NotFound | Self::Internal | Self::Status(_) => (None, Vec::new()),
        };
        let problem = ProblemDetails {
            kind: "about:blank".to_owned(),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            detail,
            errors,
        };
        // ProblemDetails holds only strings and integers, so serialization cannot fail.
        let body = serde_json::to_vec(&problem).unwrap_or_default();
        let mut response = (
            status,
            [(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON))],
            body,
        )
            .into_response();
        response.headers_mut().extend(extra_headers);
        response
    }
}

/// Rewrites error responses that did not come from `ApiError` (framework and tower layers)
/// into Problem Details. `Retry-After` survives the rewrite.
pub async fn problem_details(response: Response) -> Response {
    let status = response.status();
    let is_error = status.is_client_error() || status.is_server_error();
    let is_problem = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(PROBLEM_JSON.as_bytes()));
    if is_error && !is_problem {
        let retry_after = response.headers().get(header::RETRY_AFTER).cloned();
        let mut rewritten = ApiError::Status(status).into_response();
        if let Some(value) = retry_after {
            rewritten.headers_mut().insert(header::RETRY_AFTER, value);
        }
        rewritten
    } else {
        response
    }
}

/// A JSON body, parsed and validated with `garde`. Failures are Problem Details.
pub struct ValidJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate<Context = ()>,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        // The rejection text may quote request values (e.g. a password), so only its kind is used.
        // A body that cannot be read as the expected JSON is a 422 like a bad field (ADR-0018);
        // a missing or wrong media type is 415 and a body that is too large is 413.
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|rejection| match rejection {
                JsonRejection::JsonSyntaxError(_) => ApiError::Unprocessable {
                    detail: "the request body is not valid JSON".to_owned(),
                    errors: Vec::new(),
                },
                JsonRejection::JsonDataError(_) => ApiError::Unprocessable {
                    detail: "the request body does not have the expected shape".to_owned(),
                    errors: Vec::new(),
                },
                other => ApiError::Status(other.status()),
            })?;
        value.validate()?;
        Ok(Self(value))
    }
}

/// A query string, parsed and validated with `garde`. Failures are Problem Details.
pub struct ValidQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ValidQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate<Context = ()>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        // Like a JSON body with a bad field, a query that cannot be read is a 422.
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError::Unprocessable {
                detail: "the query string is invalid".to_owned(),
                errors: Vec::new(),
            })?;
        value.validate()?;
        Ok(Self(value))
    }
}
