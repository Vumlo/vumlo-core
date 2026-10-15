# 0012. One API for web, mobile and CLI

Status: Accepted

## Context
Web, mobile and CLI clients run at the same time. Mobile apps cannot be force-updated, and mobile networks are unreliable.

## Decision
- One REST API under `/api/v1`. No client-specific endpoints.
- Within v1 changes are additive only: new fields and endpoints, never removals or renames.
- Errors use RFC 9457 Problem Details; validation errors add `errors: [{field, message}]` (ADR-0018).
- Lists use cursor pagination: `?cursor=&limit=` → `{items, next_cursor}` (ADR-0018).
- Non-idempotent POSTs accept an `Idempotency-Key` header.
- An OpenAPI spec is generated from Rust types. The web and mobile clients are generated from it; the Rust CLI uses the `api-types` crate directly.
- Auth: cookie for web, bearer token for mobile and CLI (ADR-0003).
- Two command-line tools: `learnly-server` (the server binary; admin subcommands with direct DB access) and `learnly` (the user CLI; API only).

## Consequences
- The API contract is the product, so breaking it needs `/v2`.
- The idempotency key needs a small table and expiry.
