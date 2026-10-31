# File uploads: design

Status: draft for review. Path: architectural. Delivery: one spec, four phases.

## 1. Goal

Today nothing can be uploaded. The ADRs reserve the places (ADR-0001 `files` module,
ADR-0008/0023 `file` assignment type, ADR-0024 `file` item kind, ADR-0025 `image` and
`attachments`), but no code, table or dependency exists.

Deliver:

1. A `file` assignment type: students submit files, teachers preview them in grade mode.
2. Files in the rich-text editor (RTE), for teachers and, in text answers, for students.
3. A `file` item kind: a file (PDF, PPTX, ...) as lesson material.
4. Opening rules:
   - Browser: a PDF opens in a new tab; anything else downloads.
   - Desktop: download to Downloads, then open in the default app; if opening fails or the
     type is not safe, the file stays in Downloads (and is revealed in the folder).

Out of scope: converting Office files, antivirus scanning, quotas per user, resumable or chunked
upload, multi-file requests, the institution logo (ADR-0044; `files` is built so `org` can use it
later), video blocks backed by uploads (ADR-0027).

## 2. Decisions

| Topic | Decision |
|---|---|
| Layering | New crate `files` between `events` and `auth`: storage and metadata only. No routes, no knowledge of learning. Routes, authorization, reference tables and types live in `learning`. |
| Transport | Raw bytes, one file per request. No multipart. |
| Max size | 50 MiB per file, `[files].max_size`. |
| Grade-mode preview | PDF, images, plain text (up to 1 MiB) inline. Everything else is a card with "Open". |
| `file` assignment | Config `{max_files, accept?}`; content `{files, comment?}`. |
| Editor | `image` block (all profiles) and new `file` block (`full`, `student`). `attachments` stays empty. |
| Desktop auto-open | Only safe types; others are revealed in the folder, never executed. |

## 3. `files` crate

Position in the chain: `... -> org -> auth -> files -> events -> access -> common`. `files` may
depend only on the crates below it, and `auth`, `org` and `learning` may all call it.

### 3.1 Table

```
files.files (
  id          uuid primary key,          -- v7
  uploaded_by uuid not null,             -- user id, no FK (cross-schema)
  name        text not null,             -- original name, display only
  mime        text not null,             -- as sniffed by the server, see 3.4
  size        bigint not null check (size > 0),
  sha256      text not null,
  created_at  timestamptz not null default now()
)
```

The storage key is derived from `id` (`ab/cd/<uuid>`), never from `name`. `name` is stripped of
path separators and control characters and limited to 200 characters before insert.

### 3.2 API (public surface of `files/mod.rs`)

- `put(storage, id, data, max_size) -> Stored { id, size, sha256, head }`: takes the whole body
  (the route reads it with a `max_size` cap, so an oversized upload is refused before anything is
  stored). Streaming an upload needs a stream crate, which ADR-0014 does not list; the ceiling is
  up to 50 MiB of memory per concurrent upload. Download is streamed. Called **before** `begin`.
- `insert(conn, &Stored, uploaded_by, name, mime) -> File`: writes the metadata row inside the caller's transaction.
- `get(conn, id) -> File` and `open(storage, &File) -> ByteStream`.
- `delete(conn, storage, id)`.
- `candidates(conn, older_than) -> Vec<Uuid>`: files created before the cutoff. It cannot know
  whether anything references them; see 3.5.

### 3.3 Storage backend

`object_store` 0.14 (already listed in ADR-0014). `LocalFileSystem` at `<base_dir>/files` by
default; S3-compatible when `[files].storage = "s3"`.

Config keys (each marked `public`, `private` or `secret`, ADR-0054):

- `files.storage` (public): `local` | `s3`
- `files.path` (private): local directory
- `files.max_size` (public): bytes, default 52428800
- `files.s3.endpoint`, `files.s3.bucket`, `files.s3.region` (private)
- S3 access key and secret: `secret`, read from the environment only.

If the local backend needs `tokio` feature `fs`, it is listed in the plan (ADR-0014 rule: allowed
without an ADR when needed for a use already described).

### 3.4 Content type

The client's `Content-Type` is a hint only. The server derives `mime` from the file's leading
bytes for the **safe inline set** (PDF, PNG, JPEG, GIF, WebP, plain text) and from the extension
for everything else. If the claimed type is in the inline set but the bytes do not match, the file
is stored as `application/octet-stream`. This is hand-written signature matching (a few magic
numbers); no new crate.

### 3.5 Orphans

`files` sits below `learning` and may not read learning's reference tables. So the sweep is a flow
in `learning` (`learning::files::sweep`): take `files::candidates(now() - 24h)`, drop the ones that
appear in `content.item_files` or `assessment.submission_files`, call `files::delete` for the rest.
It runs as a periodic task started next to the outbox worker in `server` (ADR-0007: cleanup reuses
the worker). The row is deleted in a transaction, then the object. A failed object delete leaves an
object with no row: it is unreachable and harmless, and cleaning those is out of scope.

## 4. Upload and download (in `learning`)

### 4.1 `POST /api/v1/files?for=item:{id}|assignment:{id}&name=<filename>`

- Body: the raw file. `Content-Type`: a hint.
- Authorization happens **before the body is read**:
  - `for=item:{id}`: `EditContent` on the item's course (archived courses refuse it).
  - `for=assignment:{id}`: `SubmitWork` on the assignment (409 in an archived course).
- The item or assignment must already exist. (The editor works on an existing item; a new draft
  item is created before its body is edited.)
- Limit: its own request-body limit of `files.max_size`. This route skips `idempotency/` and
  `conditional.rs`, which buffer bodies up to 2 MiB and 32 MiB.
- Flow: authorize -> `files::put` (no transaction) -> `begin` -> `files::insert` -> commit.
- Response `201`: `{id, name, mime, size}`. Over the limit: `413` Problem Details.
- The new file is unreferenced until a document or submission that names it is saved.

### 4.2 `GET /api/v1/files/{id}`

Allowed if **any** of these holds, otherwise `404` (never `403`):

| Owner | Rule |
|---|---|
| Item, published version (`item_files.published = true`) | `ViewContent` |
| Item, working version | `ManageContent` |
| Submission | `ViewOwnSubmission` for the author; `ViewSubmissions` for staff, and never a `draft` submission for staff (AGENTS.md §6) |
| Unreferenced | the uploader only |

A file of a draft item is never served to a student even if its id is guessed (ADR-0025).

Response:

- `Content-Disposition: inline` only for the safe set from 3.4 (SVG is never inline); every other
  type is `attachment; filename*=UTF-8''<name>`.
- Always `X-Content-Type-Options: nosniff` and `Content-Security-Policy: sandbox`.
- `ETag: "<sha256>"`; the body is streamed, never buffered.
- Range requests are supported so that a PDF viewer can seek.

## 5. Reference tables

- `content.item_files (item_id, file_id, published bool, primary key (item_id, file_id, published))`.
  Rewritten in the same transaction as every write of an item's body (working copy) and on
  publish (published copy), by the service function that owns the write. An item's `file` kind
  and its blocks both count.
- `assessment.submission_files (submission_id, file_id, primary key (submission_id, file_id))`.
  Rewritten on draft save and on submit.

Both tables are written only by their own schema's service. Each is covered by a visibility test.

## 6. Documents (ADR-0025, ADR-0027)

- New block `image {file_id, alt?, caption?}`: allowed in every profile.
- New block `file {file_id}`: profiles `full` and `student`.
- `attachments` stays; the "must be empty" rule is unchanged.
- The parser (`content/document/parse.rs`) accepts a `file_id` only if the file exists and
  either already belongs to this owner (a row in the reference table) or was uploaded by this
  actor and is still unreferenced. Anything else is `422` with the field path. This stops a user
  borrowing someone else's file id.
- The empty-document check counts `image` and `file` blocks as content (closes the Known limit in
  ARCHITECTURE.md).
- `Feature` gains `Image` and `File`; the editor profiles in `profile.rs` follow the table above.
- A document is still limited to 2 MiB as JSON; file bytes do not count.

## 7. Item kind `file` (ADR-0024)

- `ItemKind::File`; migration extends the CHECK on `content.items.kind`.
- Body: `{"file_id": uuid, "description"?: document}`. The description uses the `full` profile.
- `create_item` takes an explicit `kind` (additive in the API; today it is inferred from the
  presence of an assignment).
- Visibility, publishing and `publish_at` are those of `text` items. No new `Action` (ADR-0024).
- Publishing copies the working body to the published slot and rewrites `item_files` with
  `published = true`.

## 8. Assignment type `file` (ADR-0008, ADR-0023)

- `BuiltinType::File`; no migration (the type column is plain text).
- Config: `{max_files: 1..=20 (default 5), accept?: [".pdf", ".docx", ...]}`. `accept` entries are
  lowercase extensions with a leading dot.
- Submission content: `{"files": [file_id, ...], "comment"?: document}` (`student` profile).
- `validate_submission`:
  - `Stage::Draft`: `files.len() <= max_files`; every file's extension is in `accept` when set
    (case-insensitive); every `file_id` passes the ownership rule from section 6.
  - `Stage::Submit`: additionally at least one file.
- No auto-grade: graded by hand.
- Events keep their ids-only payloads; saving a draft emits none (AGENTS.md §7).

## 9. Web

### 9.1 `web/src/lib/files.ts`

- `uploadFile(file: File, target): Promise<FileRef>`: browser path uses `fetch` with raw body;
  desktop path calls `invoke("upload_file", ...)`.
- `openFile(ref)`:
  - Browser: PDF -> `window.open(url, "_blank")`; others -> an `<a download>` click.
  - Desktop: `invoke("open_file", { id })`.
- Detection uses the existing `isDesktop()` (`web/src/lib/api/client.ts:30`).

### 9.2 Editor

- Slash menu entries "Image" and "File"; paste and drag-and-drop of files.
- While uploading, a placeholder block is shown; on failure it is removed and a toast explains
  (too large, type not allowed).
- `BlockRenderer` renders `image` as `<img>` and `file` as a card (icon, name, size, "Open").

### 9.3 Pages

- `ItemView`: a `file` item shows its description and an "Open" card; the course list shows a file
  icon row (`ItemTile`).
- `FileAssignmentPage` (student): drop zone, file list with remove, comment editor, autosave
  through the existing draft path, submit.
- `SubmissionView` (grade mode), for type `file`: list of files, selected one shown inline:
  PDF in an `<iframe>`, images in `<img>`, text in `<pre>` (fetched as text, up to 1 MiB);
  anything else is a card calling `openFile`. On desktop Linux (WebKitGTK) PDF falls back to the
  card, because that webview cannot render PDFs.
- `AssignmentSettings`: `max_files` and `accept` fields.
- i18n keys in both locales; MSW handlers for `POST /files` and `GET /files/{id}` in
  `web/src/mocks/handlers/`, kept out of production builds by the existing mock stubs.

## 10. Desktop (ADR-0056, ADR-0061)

The window never touches the network or the file system; Rust does. Two new commands and one URI
scheme, and ADR-0061 is amended to list them (the window still gets no `fs`, `shell` or `opener`
permission).

- `upload_file(target, name, bytes)`: receives raw bytes over Tauri IPC and sends `POST /files`
  with the stored token.
- `open_file(id)`:
  1. `GET /files/{id}` and write to the system Downloads directory. If the name exists, append
     ` (1)`, ` (2)`, ... The name is sanitized as in 3.1.
  2. If the extension is in the safe-open list (documents, spreadsheets, presentations, text,
     images, audio, video, archives): `opener::open_path`.
  3. Otherwise `opener::reveal_item_in_dir` (file selected in the folder, not run).
  4. If either step fails, return an error the web layer shows as a toast; the file stays saved.
- URI scheme `vumlo-file://{id}`: Rust fetches with the token and streams the body, so `<img>` and
  `<iframe>` work. CSP in `tauri.conf.json` gets `vumlo-file:` in `img-src` and `frame-src`.
  Only `{uuid}` paths are accepted.
- The safe-open list lives in one Rust constant with a test; executables and scripts (`exe`, `bat`,
  `cmd`, `com`, `sh`, `app`, `lnk`, `scr`, `msi`, `jar`, `desktop`, ...) are never in it, and the
  list is an allowlist, so unknown extensions are revealed, not opened.

## 11. ADRs and docs to change

New ADR `files`: sections 3 to 5. Amendments:

- ADR-0014: confirm `object_store` features actually used; `tokio` `fs` if needed.
- ADR-0023: `file` config and content (section 8).
- ADR-0024: `file` item body is built (section 7).
- ADR-0025 and ADR-0027: `image` and `file` blocks (section 6).
- ADR-0061: desktop commands and URI scheme (section 10).
- `ARCHITECTURE.md`: crate chain, module table (`files` gets an owner), Known limits.
- `AGENTS.md` section 4: add `files` to the chain; `backend/crates/server/tests/architecture.rs`
  moves `files` from `OTHER_SCHEMAS` to an owned schema.

## 12. Phases (each mergeable)

1. **Core**: `crates/files`, config, migration for `files.files`, upload and download routes with
   header rules and Range, orphan sweep, `files.ts`, desktop commands and scheme, ADR `files`.
   Since no owner exists yet, this phase's routes are covered by tests using a test-only owner.
2. **Lesson material**: `file` item kind, `content.item_files`, `create_item` `kind`, web row and page.
3. **File assignment**: `BuiltinType::File`, `assessment.submission_files`, student page, grade-mode
   preview.
4. **Editor**: `image` and `file` blocks, parser and profiles, `BlockEditor`, `BlockRenderer`, empty
   document rule.

## 13. Tests

Backend (`db_test`; API tests start the real server and use `reqwest`):

- Upload then download returns identical bytes; `sha256` and `size` are correct.
- Over `max_size` -> `413` and no object left in storage.
- Student requests a file of a draft item, or of an item not yet `publish_at` -> `404`.
- Student requests another student's submission file -> `404`; staff request a `draft` submission's
  file -> `404`.
- A user cannot save a document or submission naming someone else's file id -> `422`.
- An unreferenced file is readable only by its uploader.
- `inline` vs `attachment`: a `.svg` and a spoofed `.pdf` that is not a PDF are `attachment` /
  `application/octet-stream`; `nosniff` and `sandbox` CSP are always present.
- `max_files` and `accept`, including case-insensitivity; submit needs at least one file.
- Orphan sweep deletes only unreferenced files older than 24 h and uses a fixed clock, never sleep.
- Publishing an item moves its files to `published = true`; unpublished changes stay 404 for students.
- Empty-document rule counts image and file blocks.
- Architecture test: `files` crate has no dependency on `learning`, and no SQL outside `repo.rs`.

Web: unit tests for `openFile` branching (PDF vs other, browser vs desktop) and for the editor
upload state (placeholder, failure removal); `npm run typecheck` and `npm run build`.

Desktop: unit tests for the safe-open allowlist, the unique-name function and the sanitizer; the
URI-scheme path validation.

Manual: mock mode and a real server in local Chromium; on desktop open a PDF and a DOCX, and
confirm a `.sh` file is only revealed.

## 14. Open assumptions (to confirm at review)

- Sweep as a `learning` flow (3.5) instead of a function inside `files`, because of the
  no-reads-from-above rule. This differs slightly from the earlier chat summary.
- `files` placed directly under `auth` in the chain. If `auth` ever needs avatars this still works.
- The upload requires an existing item or assignment. If the editor has a "new item, not yet saved"
  state, that state must create the draft item first.
