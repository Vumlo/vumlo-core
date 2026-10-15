# 0007. Domain events via transactional outbox

Status: Accepted

## Context
Actions have side effects: emails, webhooks, later push notifications and analytics. Doing them inside the request makes requests slow and fragile, and it can lose effects when the process crashes between the DB commit and the send (the dual-write problem).

## Decision
- A state change and its event are inserted **in the same transaction** (`events.outbox`), through one function `events::emit(&mut tx, Event)`.
- A worker (a tokio task in the same binary) claims events with `FOR UPDATE SKIP LOCKED` and dispatches them to internal handlers and to webhook subscriptions.
- Webhooks: HMAC-signed, per-subscription delivery state in `webhook_deliveries`, exponential backoff.
- Delivery is **at-least-once**, so consumers must deduplicate by event id.
- The payload contains ids only (plus event id, type, version, `occurred_at`). Consumers fetch details through the API.
- **Draft saves (grades, content, submissions) emit no events.**
- The same worker runs scheduled jobs (`publish_at`, deadline auto-submit, cleanup).

## Consequences
- Requests do not depend on SMTP or external services. No event is lost once committed.
- Side effects are delayed by a few seconds.
- Event types are a public contract: add fields, never change their meaning.
