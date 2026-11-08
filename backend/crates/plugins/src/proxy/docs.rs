//! The proxy route in the OpenAPI document. The route itself is one axum route for any method; this
//! describes it once per method, as functions nobody calls.

use api_types::ProblemDetails;
use utoipa::OpenApi;

macro_rules! described {
    ($name:ident, $method:ident) => {
        /// A route of a plugin's own server, through the core (ADR-0078). The plugin declares its
        /// routes and who may call each in its manifest; one that is not declared, or is declared
        /// for another method, is `404`. The core forwards the call with a signed statement of who
        /// the caller is (`X-Vumlo-User`, `X-Vumlo-Role`, `X-Vumlo-Course`, `X-Vumlo-Course-Role`,
        /// `X-Vumlo-Timestamp`, `X-Vumlo-Nonce`, `X-Vumlo-Signature`) and no cookie. The answer of
        /// the plugin is passed on if it is JSON, text, CSV, iCalendar, PDF or XLSX; any other
        /// content type, a redirect, or a `5xx` is a `502`.
        ///
        /// A route the plugin declared with `auth = "none"` (ADR-0080) is for anyone: it needs no
        /// session, only answers `GET`, and the core checks nothing about the caller. The call is
        /// forwarded without `X-Vumlo-User`, `-Role`, `-Course` and `-Course-Role`, with
        /// `X-Vumlo-Anonymous: 1`, and the signature says that nobody signed in. The plugin checks
        /// its own token. Every public route has an allowance of calls shared by all who call it;
        /// over it is `429` with `Retry-After`.
        #[utoipa::path(
            $method,
            path = "/plugins/{plugin_id}/{path}",
            tag = "plugin-routes",
            security((), ("cookie" = []), ("bearer" = [])),
            params(
                ("plugin_id" = String, Path, description = "The id of the plugin"),
                ("path" = String, Path, description = "The route on the plugin's server, without its leading slash"),
            ),
            responses(
                (status = 200, description = "The plugin's answer (any 2xx or 4xx it gave)"),
                (status = 401, description = "Not logged in (a public route needs no session)", body = ProblemDetails,
                    content_type = "application/problem+json"),
                (status = 403, description = "The route is not for this caller's role, or the plugin does not reach the course",
                    body = ProblemDetails, content_type = "application/problem+json"),
                (status = 404, description = "No such plugin, route or course, or the course is not visible to the caller",
                    body = ProblemDetails, content_type = "application/problem+json"),
                (status = 413, description = "The body is more than 10 MiB", body = ProblemDetails,
                    content_type = "application/problem+json"),
                (status = 429, description = "A public route was called more than its allowance",
                    body = ProblemDetails, content_type = "application/problem+json"),
                (status = 502, description = "The plugin could not be reached or answered something the core does not pass on",
                    body = ProblemDetails, content_type = "application/problem+json"),
                (status = 504, description = "The plugin took more than 30 seconds", body = ProblemDetails,
                    content_type = "application/problem+json"),
            )
        )]
        #[allow(dead_code)]
        fn $name() {}
    };
}

described!(proxy_get, get);
described!(proxy_post, post);
described!(proxy_put, put);
described!(proxy_patch, patch);
described!(proxy_delete, delete);

#[derive(OpenApi)]
#[openapi(paths(proxy_get, proxy_post, proxy_put, proxy_patch, proxy_delete))]
struct ProxyDoc;

/// The paths of the proxy, to merge into the document of the API under `/api/v1`.
pub fn openapi() -> utoipa::openapi::OpenApi {
    let mut doc = ProxyDoc::openapi();
    // The paths are relative to the API, like every other.
    doc.paths.paths = doc
        .paths
        .paths
        .into_iter()
        .map(|(path, item)| (format!("/api/v1{path}"), item))
        .collect();
    doc
}
