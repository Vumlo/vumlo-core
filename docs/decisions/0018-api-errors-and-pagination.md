# 0018. API validation errors and cursor pagination

Status: Accepted
Supplements: ADR-0012

## Context
ADR-0012 requires RFC 9457 Problem Details and cursor pagination, but fixes no shape. Task 002 needs both: a weak password must return zxcvbn's feedback, and `GET /admin/users` is the first list.

## Options
- Errors: a generic `errors` list; a dedicated `password_feedback` member; text only in `detail`.
- Cursor: an opaque id cursor; a base64 keyset cursor (flexible sort); no pagination for small lists.

## Decision
- A validation failure is `422` Problem Details with an extension member `errors: [{ "field": string, "message": string }]`. The member is omitted when empty. A password rejection lists zxcvbn's warning and each suggestion as entries with the password field name.
- Lists accept `?cursor=<opaque>&limit=<n>` (default 50, max 100) and return `{ "items": [...], "next_cursor": string | null }`. Items are ordered by `id` (UUID v7, so creation order). Clients must treat the cursor as opaque; today it is the last returned id.

## Consequences
- One error shape for garde and domain validation; adding members to Problem Details is additive.
- Sorting by anything other than creation order needs a new cursor format, which stays compatible because the cursor is opaque.
