# 0003. Identity, login and sessions

Status: Accepted

## Context
Users sign in with an institutional email + password, or with Google (Microsoft later). The same backend serves web, mobile and CLI. Previously auth was outsourced to Supabase.

## Options
- JWT — stateless, but revocation is hard and secrets have to rotate.
- **Opaque session tokens stored in PostgreSQL** — trivially revocable, one table, and they work as a cookie or as a bearer token.

## Decision
- `users` is separate from `identities`. A user can have several login methods; a password is just the `password` provider.
- Registration is open only for emails in `allowed_domains`. New users get role `student`. An admin promotes teachers.
- A Google identity is linked to an existing user **only if Google reports `email_verified = true`** and the domain is allowed.
- Sessions are random tokens; only their hash is stored. `kind = web | app | cli | pat`. Users can list and revoke their sessions.
- Delivery: an HttpOnly, SameSite cookie for web (plus an `Origin` check against CSRF); `Authorization: Bearer` for mobile and CLI.
- CLI login: device authorization flow (RFC 8628) from v1. The CLI shows a code, the user approves it in the browser, and the CLI receives a `cli` session. State lives in `identity.device_authorizations`. Personal access tokens (`pat`) exist for scripts and CI, not as the login method.
- Mobile Google login: the app sends the Google ID token, and the server verifies it and issues its own session.
- First admin: `learnly create-admin <email>` (server CLI), or set by hand in the database.

## Consequences
- Every request does one indexed lookup by token hash. That is acceptable.
- Password hashing, rate limiting of login and email verification are our responsibility now.
