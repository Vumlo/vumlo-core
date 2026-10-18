# 0047. Google sign-in (the web redirect flow)

Status: Accepted
Supplements: ADR-0003, ADR-0017, ADR-0038, ADR-0046

## Context
ADR-0003 and ADR-0017 plan sign-in with Google for the web: only for an address in `allowed_domains`,
only with `email_verified = true`, and a new account is a `student`. ADR-0014 lists `openidconnect` 4
for it (discovery, PKCE, ID-token verification). The web app starts the flow with
`POST /auth/google/start` and finishes it on its own callback page with `POST /auth/google/callback`.

Two attacks need an answer in the design. A *login CSRF* gives a victim's browser the attacker's
authorization code, so the victim ends up signed in as the attacker. A *pre-claimed account* is an
unverified self-registration (ADR-0046) of the victim's address with the attacker's password; if Google
sign-in simply linked to it, the attacker's password would keep working on the victim's account.

## Options
- Where the state of one attempt lives: (a) a signed cookie: needs a signing key we do not have;
  (b) **a row in `identity.oauth_states`**, found by the hash of the `state` and bound to the browser by a
  second random value in a cookie.
- Discovery: (a) at startup: the server cannot start while Google is unreachable; (b) **on first use,
  kept in memory**, a failure is `503` and the next request tries again.
- Linking to an existing account: (a) never: a person with a password account could not use Google;
  (b) **link by the verified address, and take the password away from an account whose address was
  never verified**.

## Decision
1. `[auth.google]` is optional: `client_id`, `client_secret` (redacted in `Debug`), `redirect_uri` (an
   absolute URL, the web callback page) and `issuer` (default `https://accounts.google.com`). Without it
   both endpoints are `404`.
2. `POST /auth/google/start {login_hint?}` makes a state, a nonce and a PKCE verifier (S256), stores
   them in `identity.oauth_states` for 10 minutes together with the hash of a random browser value, sets
   that value as the cookie `learnly_oauth` (HttpOnly, SameSite=Lax, path `/api/v1/auth/google`, 10
   minutes) and answers `{authorization_url}`. The scopes are `openid email profile`. Expired rows are
   deleted when a new one is written. The nonce and the verifier are stored as they are: they live ten
   minutes and are useful only with the authorization code and the browser's cookie.
3. `POST /auth/google/callback {code, state}` first applies the Origin check of a web login. It then
   deletes the row that matches the hash of `state`, the hash of the cookie and a time in the future,
   in one statement; no row, or no cookie, is `401` ("the sign-in attempt is not valid"), so a state is
   single use and cannot be replayed from another browser. The code is exchanged and the ID token
   verified (signature, issuer, audience, expiry, nonce, by the library) with no transaction open and
   with an HTTP client that follows no redirect. A failure of Google or of the exchange is `401`; an
   unreachable Google is `503`.
4. The token must carry an email with `email_verified = true`, and its domain must be one of
   `auth.allowed_domains` (an empty list allows nobody): otherwise `403`.
5. One transaction resolves the account: the identity `(google, sub)` is the user; otherwise the user
   with that address gets the identity linked and `email_verified_at` set; if that user had never
   verified the address, their password identity, unused tokens and sessions are deleted (a pre-claimed
   account); otherwise a verified `student` is created with no password identity. A disabled user is
   `403`. The first name and surname come from `given_name` and `family_name`, else from the part of the
   address before `@` and `Student`, cut to 100 characters. Signing in with Google is not audited
   (ADR-0034, point 2).
6. The session is a `web` session created like a password login: the `learnly_session` cookie,
   `Cache-Control: no-store`, the user in the body, and the `learnly_oauth` cookie cleared. A session
   for a user without a password locks the user `FOR SHARE` and checks `status`, like ADR-0038.
7. Both endpoints sit behind the per-client limiter of ADR-0046. They are not counted per address: the
   address is not known before Google answers.
8. Tests run against a fake issuer in the test crate (discovery, an empty key set, a token endpoint that
   checks the PKCE verifier). It advertises `HS256` and signs ID tokens with the client secret, which the
   library accepts, so no RSA key and no new crate is needed; the signature check itself is the
   library's, and this ADR does not retest it.
9. Not decided here: the mobile flow that sends a Google ID token (ADR-0003), Microsoft, unlinking an
   identity, and showing which identities an account has.

## Consequences
- A stranger cannot make a victim's browser finish the stranger's sign-in, and a replayed or expired
  state is `401`.
- A person who registered with a password and then uses Google loses nothing but an unverified password.
- The first Google sign-in after a restart pays for the discovery request.
- `reqwest` stays at 0.12 and comes through `openidconnect`'s re-export; the server crate takes no
  direct dependency on it.
- A cookie-less client (a script) cannot use this flow. That is intended: it is the browser flow.
