# 0058. `Idempotency-Key` on POST

Status: Accepted
Amends: ADR-0012, ADR-0039 (decision 6), ADR-0041 (decision 3)

## Context
ADR-0012 promised that non-idempotent `POST`s accept an `Idempotency-Key`, and ADR-0039 deferred it.
The desktop app (ADR-0056, ADR-0060) keeps a send queue: after a dropped connection it cannot know
whether a `POST` reached the server, so it sends it again. Without a key, a submit that did arrive is
answered `409 "there is no draft to submit"` and a create makes a second row.

## Options
1. **Per endpoint.** Each handler takes the key and stores its own result. AGENTS §9 forbids ad-hoc
   handling, and 40 handlers would each get it slightly differently.
2. **One layer around the API** that records the answer of the first request and replays it.
3. **Natural keys** (a client-chosen id in each body). It changes every request type.

## Decision
1. **Option 2.** `server/src/idempotency/` is a layer, innermost around the whole router. Handlers do not
   know it exists. The header is optional; a request without it is untouched.
2. **What is covered:** a `POST` under `/api/v1/` except `/api/v1/auth/*`. Sign-in, registration and
   password calls answer with credentials, which are never kept (AGENTS §11). `PUT` and `DELETE` repeat
   harmlessly by themselves, and `PATCH` is a replace of named fields. The draft save of a student is a
   `PUT` and is last-write-wins already (ADR-0035).
3. **The key** is 1 to 255 printable ASCII characters, sent once. Anything else is `422` on the field
   `Idempotency-Key`. Clients use a UUID.
4. **Scope.** A key belongs to the credential that sent it: the table holds the SHA-256 of the bearer
   header or the session cookie, never the token. Another person, or the same person with another
   session, cannot meet the key. A request with no credential is passed through (it will be refused).
5. **Table** `idempotency.keys`, schema `idempotency` (a new module, AGENTS §4): primary key
   `(scope_hash, key)`, `request_hash` (SHA-256 of path, query and body), `status`, `content_type`,
   `body`, `started_at`, `expires_at` (24 hours). Records expire on the way in, as `oauth_states` do.
6. **Behaviour.**
   - First request: a row is inserted, the request runs, and the answer is stored. This runs on its own
     task, so a client that disconnects does not leave a row that never finishes.
   - The same key and the same request again: the stored status, content type and body, plus
     `Idempotency-Replayed: true`.
   - The same key for another path or body: `422` on `Idempotency-Key`.
   - The same key while the first request is running: `409` with `Retry-After: 1`.
   - A row still running after two minutes, longer than any request (30 s timeout), belongs to a process
     that is gone; a retry takes it over and runs the request.
7. **What is kept:** every `2xx` and every `4xx` except `401`, `408` and `429`. A refusal such as
   `409 "the submission window is closed"` is an answer about the request, and the same key gets it
   again; a new attempt takes a new key. Server errors, timeouts, throttles and refused credentials say
   nothing about the request: the row is deleted and the retry runs. Because a `401` is never kept, a
   caller with a bad token cannot fill the table.
8. **CORS.** `Idempotency-Key` and `If-None-Match` are allowed request headers and `Idempotency-Replayed`
   and `ETag` exposed (ADR-0041 decision 3, which allowed neither).

## Consequences
- The record is separate from the transaction of the request: the layer writes it with single
  statements on its own connection. If the process dies after a handler commits and before the answer is
  stored, the retry waits two minutes and runs the request again. That window is a crash, not a retry.
- A client must generate a new key for each logical action and reuse it for every retry of that action.
- The OpenAPI document does not list the header on each operation; this ADR is where it is described.
- A POST with a very large answer is not stored (the limit is 8 MiB; no answer comes close).
