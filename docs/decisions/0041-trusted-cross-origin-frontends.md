# 0041. Trusted frontends on another origin

Status: Accepted
Amends: ADR-0003, ADR-0012, ADR-0013

## Context
Our own `web/` is served on the same site as the API behind a reverse proxy (ADR-0013). An institution
may instead host its own browser frontend on another origin and call the API directly (ADR-0012: one
API for every client). That fails today for two reasons: the API sends no CORS headers, and the CSRF
rule of ADR-0003 refuses a cookie-authenticated request that is not read-only unless its `Origin` is
`server.public_origin`. Allowing CORS alone would still leave every cookie write at `403`, so both
change together.

## Options
- Allow any origin (`*` or a reflected `Origin`). With credentials this lets any website act as the
  signed-in user. Rejected.
- Patterns or wildcards for subdomains. A pattern widens silently when a subdomain is taken over.
  Rejected.
- **An operator-owned list of exact origins.** One setting drives both CORS and the CSRF check.

## Decision
1. **`server.allowed_origins`** is a list of exact origins (`scheme://host[:port]`), empty by default.
   Empty means today's behavior: no CORS headers at all. There is no `*` and no pattern.
2. **Every origin key is checked at startup**, `server.public_origin` included: `http` or `https`, a
   lowercase host, an optional port that is not the scheme's default, no path, query or fragment (one
   trailing `/` is dropped). A browser sends its `Origin` in exactly this form, so an entry in any
   other form could never match. An invalid entry stops the server and names the key and the value.
3. **CORS, when the list is not empty.** Only a listed origin gets `Access-Control-Allow-Origin` (its
   own value) with `Access-Control-Allow-Credentials: true`, because the web session is a cookie.
   Methods `GET, POST, PUT, PATCH, DELETE`; request headers `Authorization` (browser-based bearer
   clients) and `Content-Type`; exposed response header `Retry-After` (`429`, `503`); preflight cached
   for 10 minutes. `Idempotency-Key` is not allowed: no endpoint reads it (ADR-0039 decision 6), and
   adding it later is additive.
4. **Preflight is answered before everything else.** The CORS layer is the outermost layer, so an
   `OPTIONS` preflight needs no session and meets no CSRF check and no rate limit, and the headers
   also reach error responses that the Problem Details rewrite builds.
5. **The CSRF check accepts `server.public_origin` or any allowed origin.** It covers every
   cookie-authenticated request that is not GET, HEAD or OPTIONS, and a `web` login. Bearer requests
   stay exempt. Any other `Origin`, or none, is the same `403`.

## Consequences
- Cookies stay `SameSite=Lax` (no `SameSite=None`, not supported). Cookie authentication from another
  origin therefore works only when the frontend is **same-site** with the API (`app.uni.edu` calling
  `lms.uni.edu`). A frontend on a different site must use bearer tokens (`client: app`), which are
  sent in `Authorization` and need no cookie.
- Listing an origin lets the JavaScript served there act with the credentials of whoever uses it. It
  is a list of trusted frontends, not a switch that makes the API public. The operator is
  responsible for the code on those origins.
- `server.public_origin` is stricter than before; a value that could never have matched a browser
  `Origin` (an upper-case host, an explicit default port) now stops the server instead of refusing
  every cookie write.
- `tower-http` gets its `cors` feature (a use ADR-0014 already names).
