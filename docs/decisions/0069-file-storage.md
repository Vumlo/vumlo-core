# 0069. File storage: the `files` crate, raw-byte upload, and owner-authorized download

Status: Accepted
Amends: ADR-0001, ADR-0014, ADR-0023, ADR-0024, ADR-0025, ADR-0027, ADR-0061, ADR-0065
Supplements: ADR-0016

## Context
The ADRs reserve a `files` module (ADR-0001), a `file` assignment type (ADR-0008, ADR-0023), a `file` item kind (ADR-0024) and `image` and `attachments` in documents (ADR-0025), but nothing is decided about storage, upload, serving or the desktop app. The `files` schema exists and is empty. ADR-0025 already fixes one rule: a file is served only through the owner that uses it.

## Options
- (a) Everything in `learning/src/files/`: fewest parts, but `org` could never use it (logo, ADR-0044) without moving it.
- (b) **A `files` crate that only stores and describes bytes,** with routes, authorization and references in `learning`.
- (c) An optional module (ADR-0066): impossible, because `learning` would depend on it and a module may write only to `mod_<id>`.

## Decision
- **Crate.** `crates/files` sits between `auth` and `events` in the chain (ADR-0065). It owns `files.files(id, uploaded_by, name, mime, size, sha256, created_at)` and the storage backend. It has no routes, no `Actor` and no knowledge of items or submissions.
- **Storage.** `object_store`: local disk at `<base_dir>/files` by default, S3-compatible by config. The key derives from the id, never from the name. Config keys `files.storage`, `files.path`, `files.max_size` (default 50 MiB), `files.s3.*`; S3 credentials are `secret` and come from the environment (ADR-0054).
- **Upload.** `POST /api/v1/files?for=item:{id}|assignment:{id}&name=` with the raw file as the body, one file per request, no multipart. Authorization (`EditContent` or `SubmitWork`) happens before the body is read. The bytes are stored first (`files::put`, no transaction), then the metadata is inserted in the caller's transaction (ADR-0016). The route has its own body limit and bypasses the idempotency and conditional layers, which buffer bodies.
- **Download.** `GET /api/v1/files/{id}` in `learning` is allowed if an owner allows it: `ViewContent` for the published version of an item, `ManageContent` for the working version, `ViewOwnSubmission` or `ViewSubmissions` for a submission (never a `draft` for staff), or the uploader for a file nothing references. Otherwise `404`. `inline` only for PDF, PNG, JPEG, GIF, WebP and plain text; everything else, and SVG always, is `attachment`. Always `nosniff` and `Content-Security-Policy: sandbox`. `ETag` is the sha256, the body is streamed, and `Range` is supported.
- **Content type.** The client's type is a hint. For the inline set the server checks the leading bytes; a mismatch is stored as `application/octet-stream`.
- **References.** `content.item_files(item_id, file_id, published)` and `assessment.submission_files(submission_id, file_id)` are rewritten by the owning service in the same transaction as every write of the body or content. A document or submission may name a file only if it already belongs to that owner or the actor uploaded it and nothing references it yet (`422` otherwise).
- **Orphans.** A flow in `learning` (`files` may not read the schemas above it) deletes files older than 24 h that no reference table names. It runs as a periodic task beside the outbox worker (ADR-0007).
- **Documents.** New block `image {file_id, alt?, caption?}` in every profile and `file {file_id}` in `full` and `student`; `attachments` stays empty (amends ADR-0025, ADR-0027). The empty-document rule counts both blocks.
- **Item kind and assignment type.** Item kind `file` with body `{file_id, description?}` (ADR-0024). Assignment type `file` with config `{max_files: 1..=20 (5), accept?: [".ext"]}` and content `{files: [file_id], comment?}`, graded by hand (ADR-0023).
- **Desktop.** Two Rust commands, `upload_file` and `open_file`, and a `vumlo-file://{id}` URI scheme that fetches with the stored token. `open_file` saves to the Downloads directory (a numeric suffix if the name is taken) and opens only extensions on an allowlist of documents, images, media and archives; anything else is revealed in its folder, never run. The window still gets no `fs`, `shell` or `opener` permission (amends ADR-0061).
- **Dependencies.** `object_store` 0.14 as already listed in ADR-0014. If the local backend needs the `tokio` feature `fs`, the plan lists it. No other crate.

## Consequences
- `tests/architecture.rs` moves `files` from the ownerless schemas to an owned one, and the chain in AGENTS.md section 4 and ARCHITECTURE.md gains `files`.
- Backups: `db backup` is `pg_dump` only (ADR-0054), so the files directory or bucket must be backed up separately. This is documented, not solved here.
- A reverse proxy in front of the server must allow bodies of `files.max_size`.
- Not decided here: antivirus, per-user quotas, resumable upload, Office conversion, uploaded video blocks, and the logo.
