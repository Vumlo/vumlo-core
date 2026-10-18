# 0045. The outbox worker and mail delivery

Status: Accepted
Supplements: ADR-0007, ADR-0014, ADR-0016, ADR-0017

## Context
ADR-0007 puts every side effect behind the outbox: the state change and its event commit together
and a worker in the same binary delivers. ADR-0017 needs mail (verification and password reset links)
and ADR-0014 names `lettre` for it. The table `events.outbox` exists and two events are written to it,
but nothing reads it. Registration is the first feature that needs a delivered event.

AGENTS.md section 4 forbids slow work inside a transaction, so a handler that sends mail must not hold
the transaction that claimed its row.

## Options
- Claiming: (a) hold `FOR UPDATE SKIP LOCKED` in one transaction around the handler: the lock lives
  as long as the SMTP call; (b) **claim in a short transaction and lease the row**: the claim bumps
  `attempts` and moves `run_after` forward, the handler runs with no transaction, and a crash lets the
  lease expire.
- Where handlers live: (a) the worker knows every module; (b) **each module exposes a handler and the
  composition root registers it**: `events` stays below the modules it serves.
- Mail transport: (a) SMTP only: every test and every developer needs a mail server; (b) **SMTP and a
  file transport** (one `.eml` file per message in a directory) for development and tests, beside
  Mailpit for a visual inbox.
- Mail language: (a) English only; (b) **the institution's language** (ADR-0044), two short texts per
  mail, no templating crate.

## Decision
1. `events::run_worker` is a tokio task that `serve` starts. Every 2 seconds, and again at once while
   a batch was full, it claims up to 20 rows that are neither processed nor failed and whose
   `run_after` has passed. The claim is one `UPDATE ... WHERE id IN (SELECT ... FOR UPDATE SKIP LOCKED)`
   that sets `attempts = attempts + 1` and `run_after = now() + 5 minutes` (the lease) and returns the rows.
2. The handler of the event's type runs outside any transaction. Success sets `processed_at`. A handler
   that needs the database opens its own short transaction and calls exactly one writing service
   function (ADR-0016), the way a route handler does; it does its slow work (SMTP) after the commit.
3. A failure stores `last_error` (handlers never put secrets or personal data in an error) and sets
   `run_after = now() + min(10 s * 2^attempts, 1 hour)`. The fifth failed attempt sets `failed_at`
   instead: the row stays as a dead letter, is logged at error level with its id and type, and is not
   retried. Delivery is at least once (ADR-0007): a handler must tolerate seeing an event twice.
4. A type without a handler is marked processed. `grade.published` and `submission.submitted` are
   consumed this way until webhooks exist.
5. `events::Handlers` maps a type to a function `(PgPool, OutboxEvent) -> Result<(), HandlerError>`.
   `app` builds it from the handlers the modules expose (`identity` registers its two). Tests call
   `events::process_once` with the same registry, so they drive delivery without a timer.
6. Migration: `events.outbox` gets `attempts`, `run_after`, `last_error`, `failed_at` and a partial
   index on the pending rows.
7. `mail.rs` (top level, next to `actor.rs`) holds `Mailer`, built once in `AppState::new` from the
   optional `[mail]` section: `from`, `link_base` (default `server.public_origin`, validated like an
   origin) and `transport`, either `smtp` (`host`, `port`, `tls` = `starttls` | `tls` | `none`,
   `username`, `password`) or `file` (`dir`). The password is redacted in `Debug`. Messages are plain
   text. A bad `from`, `link_base` or host stops the server at startup.
8. Without `[mail]` the endpoints that must send mail (`POST /auth/register`, `POST /auth/password-reset`)
   answer `404`: the feature is off. Nothing else depends on mail.
9. `compose.yaml` gets Mailpit (dev only, ADR-0014) and the API container sends to it.

## Consequences
- A request never waits for SMTP, and a mail server outage delays mail instead of failing requests.
- Mail can arrive twice after a crash between the send and `processed_at`.
- A dead letter needs an operator; there is no admin screen for the outbox yet.
- Webhooks, scheduled jobs and cleanup (ADR-0007) reuse the worker and its registry later.
- Mail delivery latency is up to 2 seconds plus the SMTP time.
