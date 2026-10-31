# 0024. Item kinds: text, link, file, lesson, assignment

Status: Accepted
Amends: ADR-0012, ADR-0020
Supplements: ADR-0021

## Context
ADR-0020 built the course content tree with one item kind, `lesson`, which is a single document. Teachers also need a link, an uploaded file (a document, slides, a video) and a lesson in the sense of a short presentation made of steps (text, self-check question, link, file). The word `lesson` therefore changes meaning, and the single document becomes `text`.

## Options
- (a) Few kinds, with links and files as blocks inside documents: a bare link or file could not be listed, reordered or published as an item of its own.
- (b) One kind `material` with a `type` and `config`, like assignments: the built `lesson` would be renamed anyway, and material kinds are few and stable, unlike assignment types.
- (c) **Flat kinds,** each with a body validated in code.

## Decision
- Item kinds: `text`, `link`, `file`, `lesson`, `assignment`. `kind` stays a `text` column with a CHECK; adding a kind is a migration plus code.
- The built kind `lesson` (one document) is renamed `text`. A migration updates the rows and the CHECK, and the API enum, DTOs and tests follow. This breaks the "additive only" rule of ADR-0012 once. That is acceptable because nothing is published and no client exists; the exception is recorded here and is not repeated. After the rename `lesson` is free for ADR-0026.
- `body` stays one JSON object, shaped by the kind and validated in code:
  - `text`: a block document (ADR-0025, profile `full`).
  - `assignment`: a block document, the description (ADR-0025, profile `full`).
  - `lesson`: a lesson document (ADR-0026).
  - `link`: `{"url": ..., "embed": bool, "description"?: block document}`. The url is http or https, at most 2048 characters. `embed` shows a video inline only for hosts on the allowlist of ADR-0025. The server never fetches the url.
  - `file`: `{"file_id": ..., "description"?: block document}`. It needs the `files` module, so the kind is added in the task that builds that module.
- Visibility, publishing and ordering are the same for every kind (ADR-0010, ADR-0019, ADR-0020). No new policy actions: `ManageContent` and `ViewContent` cover every kind.
- Only `assignment` has a row in `content.assignments`. No other kind has an extra table.

## Consequences
- `text` the item kind and `text` the assignment type (ADR-0023) are different namespaces (`kind` and `assignment.type`).
- The rename is its own task and comes before any new kind.
- ADR-0021's sentence "a `lesson` has none" now reads "an item that is not an assignment has none".
- Where ADR-0020 says `lesson` it now means `text`.
- Tree summaries carry the kind; clients read the body with `GET /items/{id}`.
- Order of work: the rename, then `link`, then the lesson (ADR-0026), then `file` with the `files` module.

## Built by ADR-0069 (file uploads, item kind `file`)
- Kind `file` exists (migration `0038`). Its body is `{"file_id"?: uuid, "description"?: block document}`, nothing else. `file_id` is optional in a draft: the upload needs an existing item (`POST /files?for=item:{id}`), so a file item is created first with `{}` and `PATCH /items/{id}` names the file afterwards. `description` is validated as profile `full`; `file_id` must be a file the actor uploaded and nothing uses yet, or one the item already names.
- Publishing a file item whose body names no file (`POST /items/{id}/publish`, `POST /items/{id}/revision/publish`, `publish_module` with `cascade`) is `409` "a file item needs a file before it is published". A module publish with such an item below it publishes nothing.
- `POST /modules/{id}/items` takes an optional `kind` (`text`, `assignment` or `file`). Without it the kind is `assignment` when an `assignment` object is sent and `text` otherwise. `link` and `lesson` are still not built.
