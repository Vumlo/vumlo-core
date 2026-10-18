# 0010. Content publishing

Status: Accepted; its last decision (edits to published content are visible at once) is superseded by [ADR-0049](0049-unpublished-revisions-class-dates-and-item-meta.md)

## Context
Teachers prepare lessons and assignments in advance, finish them later, publish several at once, or schedule them.

## Decision
- Modules and items have `status: draft | published` and an optional `publish_at`.
- A student sees an item only if the course, the module and the item are published and `publish_at <= now()`. The check lives in `policy`.
- `POST /modules/{id}/publish { "cascade": bool }` — the parameter is required, with no default. The web UI asks in a dialog; other clients choose explicitly.
- Bulk publish of selected items runs in one transaction.
- `item.published` for scheduled items is emitted by the worker when `publish_at` passes.
- Edits to already published content are visible immediately. There are no draft revisions of published content in v1.

## Consequences
- Visibility is computed in queries, so no job is needed just to "open" content.
- Draft revisions of published content would reuse the two-slot approach from ADR-0009.
