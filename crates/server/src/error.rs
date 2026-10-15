use api_types::ProblemDetails;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

const PROBLEM_JSON: &str = "application/problem+json";

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    ServiceUnavailable(&'static str),
    /// A status produced outside our handlers (405, 408, 413, ...).
    Status(StatusCode),
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Status(status) => *status,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let problem = ProblemDetails {
            kind: "about:blank".to_owned(),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            detail: match self {
                Self::ServiceUnavailable(detail) => Some(detail.to_owned()),
                Self::NotFound | Self::Status(_) => None,
            },
        };
        // ProblemDetails holds only strings and integers, so serialization cannot fail.
        let body = serde_json::to_vec(&problem).unwrap_or_default();
        (
            status,
            [(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON))],
            body,
        )
            .into_response()
    }
}

/// Rewrites error responses that did not come from `ApiError` (framework and tower layers)
/// into Problem Details.
pub async fn problem_details(response: Response) -> Response {
    let status = response.status();
    let is_error = status.is_client_error() || status.is_server_error();
    let is_problem = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(PROBLEM_JSON.as_bytes()));
    if is_error && !is_problem {
        ApiError::Status(status).into_response()
    } else {
        response
    }
}
