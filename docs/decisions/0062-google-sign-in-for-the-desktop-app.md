# 0062. Sign-in with Google for the desktop app

Status: Accepted
Amends: ADR-0047, ADR-0056

## Context
ADR-0047 signs a person in with Google in the browser: the server starts the attempt and sets a cookie that
ties it to that browser; Google sends the browser back to the web app's callback page; the page finishes the
attempt and the server opens a cookie session. The desktop app has no cookie and does not run in the
browser the person signs in with, and its session is a bearer token (ADR-0003, ADR-0056).

## Options
1. **A loopback listener in the app** (RFC 8252): the app listens on `127.0.0.1:<port>` and Google redirects
   there. It needs a second Google "desktop" OAuth client, a redirect URI per port and an open port;
   a university has to register both clients.
2. **The web app's callback page, then a link to the app.** The one registered `redirect_uri` stays; the page
   hands a short code to the app through the app's own URL scheme (ADR-0061).
3. **The ID token from a Google library in the app**, as ADR-0003 sketches for mobile. It puts a Google
   client in the app and the client id in every install.

## Decision
1. **Option 2.** The registered `auth.google.redirect_uri` is unchanged, and so is the web sign-in.
2. **Start.** `POST /auth/google/start` takes an optional `app_challenge`: the SHA-256 of a secret the app
   made, as 64 lowercase hex characters (anything else is `422`). The attempt is then tied to that, not to a
   browser: no cookie is set. `identity.oauth_states` gets `app_challenge`, `browser_hash` becomes optional,
   and a constraint demands exactly one of the two.
3. **The system browser** opens the authorization address the server returned (only `http(s)`), Google
   asks, and sends the browser to the callback page as before.
4. **Callback.** For an app's attempt `POST /auth/google/callback` needs no binding cookie (a browser's still
   does, and a refused callback still does not spend its attempt). It runs the same checks (origin of the
   page, verified address, allowed domain, enabled account, the same creation or linking of the account). It
   opens **no session**. It answers `{handoff}`: a random code, stored as its SHA-256 in the new table
   `identity.app_handoffs` with the user and the challenge, for one minute. The page then goes to
   `<scheme>://auth/google?code=<handoff>` and shows a button for the same link. The scheme is in the client
   config (`desktop.url_scheme`, below).
5. **Exchange.** `POST /auth/app/exchange {code, verifier}`, in the allowance of the other sign-in calls,
   deletes the handoff whatever happens, checks `SHA-256(verifier)` against the stored challenge, checks the
   account is enabled and answers an `AppLoginResponse` exactly like `POST /auth/login` with `client: app`.
   An unknown, used, expired or mismatched code is the same `401`.
6. **In the app**, the secret lives in memory only, from `start` until the link arrives or the app stops. A
   link with no sign-in started, or a second link, finds no secret and signs nobody in.
7. **Config.** A new section `[desktop]` with `url_scheme` (private; absent: `vumlo-` and the short name in
   ASCII letters and digits, or plain `vumlo` when it has none). It must match `vumlo-` and lowercase letters,
   digits and single hyphens. The client config carries the one in use as `desktop.url_scheme`, for the page
   and for the desktop build. This replaces ADR-0056's mention of a public key.
8. **Dependencies** of the app: `getrandom` (the secret), `sha2` (its hash), `tauri-plugin-opener` (the
   system browser, from Rust only; the window is granted nothing of it).

## Consequences
- Why a link is a safe way to deliver the code: it goes to the program the operating system has registered for
  the scheme on the machine where the browser runs, and it is useless without the secret, so a program that
  merely registers the same scheme (or an attacker who starts an attempt of their own and sends the victim its
  Google address) cannot turn it into a session. The attacker's handoff would arrive on the victim's machine,
  where the secret of the attacker's attempt does not exist.
- The person has to be at the machine they sign in on with the app installed and registered for the scheme.
  An AppImage is registered by the tool that integrates it; an unsigned build may meet the operating system's
  own question about opening an app from a link.
- Signing in with Google needs the app to be running when the link arrives, since the secret is in memory. Quit
  during the sign-in and the person starts again.
- `GET /auth/google/callback` for the web is unchanged; the OpenAPI document gains the new request field,
  response and endpoint.
