# File uploads Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Students and teachers can upload files: a `file` assignment type with an in-browser grading preview, `image` and `file` blocks in the editor, and a `file` lesson-material item, with the PDF-in-new-tab / download rules in the browser and save-and-open in the desktop app.

**Architecture:** A new storage-only crate `files` (between `auth` and `events`) owns `files.files` and the `object_store` backend. Everything about learning (routes, authorization through the owning item or submission, reference tables, the orphan sweep) lives in `learning`. The web talks to it through `lib/api/client.ts`; the desktop app through two new Rust commands and a URI scheme.

**Tech Stack:** Rust (axum 0.8, sqlx, `object_store` 0.14, `sha2`), Next.js 16 / React 19, Tauri 2.12.

**Spec:** `docs/superpowers/specs/2026-10-31-file-uploads-design.md`. Decision record: `docs/decisions/done.md`.

## Global Constraints

- AGENTS.md applies in full. Layers point only downward; SQL only in `repo.rs`; `query!` only; service and repo functions take `&mut PgConnection` and never `begin`/`commit`; no `unwrap`/`expect` outside tests; unit tests live in `tests.rs` beside the file, never inline.
- Dependencies: only `object_store` 0.14 is added (ADR-0014). **No other crate**, no `futures`, `bytes`, `tokio-util`, `http-body-util`. If a step seems to need one, stop and propose an ADR.
- Max file size `files.max_size`, default `52428800` (50 MiB). A file name is at most 200 characters after cleaning; the storage key is derived from the id, never from the name.
- `Content-Disposition: inline` only for `application/pdf`, `image/png`, `image/jpeg`, `image/gif`, `image/webp`, `text/plain`; everything else (SVG always) is `attachment`. Every file response has `X-Content-Type-Options: nosniff` and `Content-Security-Policy: sandbox`.
- A file the caller may not read is `404`, never `403`.
- An item's or submission's file ids go into the reference tables in the same transaction as the write; payloads of events never carry file names.
- Migrations are append-only, use the next free number, schema-qualified tables. After any query change: `cargo sqlx prepare --workspace -- --all-targets` from `backend/`, commit `backend/.sqlx/`.
- Web: ids in the query string, addresses only from `src/lib/routes.ts`, colours only `var(--token)`, no `fetch` of the API outside `lib/api/client.ts`, no new dependency, every new screen opens under `pnpm dev:mock` in light and dark with no console errors. Web has no unit-test runner: web tests are Playwright specs in `web/e2e/` (run per `memory/web-e2e-local-chromium.md`).
- Desktop: the window gets no `fs`, `shell` or `opener` permission (ADR-0061). Executable and script extensions are never opened.
- Branch `feat/files`; Conventional Commits; no push.
- Definition of done per phase (from `backend/`): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo test --workspace -- --test-threads=16`, `cargo sqlx prepare --workspace --check -- --all-targets`, `SQLX_OFFLINE=true cargo check --workspace --all-targets`, and the same with `--features dummy`. From `web/`: `pnpm typecheck`, `pnpm build`. From `desktop/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `npm test`.

## Deviation from the spec (ruling, recorded here)

Spec 3.2 says `files::put` streams the body. Streaming an axum body without a stream crate is not possible, and the spec and AGENTS section 3 forbid a new crate. So `put` takes the whole body, which the route reads with `axum::body::to_bytes(body, max_size)`. Cost: up to 50 MiB of memory per concurrent upload. `// ponytail: buffered upload, stream it when an ADR adds a stream crate` goes on `put`. Download stays streamed (`object_store::GetResult::into_stream` into `Body::from_stream`). The route still rejects an oversized body before anything reaches storage, so no partial object is ever left.

## Review Focus

Each line below has its test in the named task.

1. A name with path parts, non-ASCII or 300 characters (`../../etc/passwd`, `Звіт з фізики.pdf`) is cleaned for storage and sent in a safe `Content-Disposition` (Task 3, Task 8).
2. A zero-byte upload is refused with `422`, not stored (Task 5).
3. Removing a file from an item body or draft submission stops it being served and makes the sweep delete it after 24 h (Task 6, Task 10, Task 13).
4. Upload to an archived course or after the submission window closed is `409`, and nothing is stored (Task 5, Task 12).
5. A `Range` request past the end is `416`; a student resubmitting the same bytes twice creates two files, one of them swept as an orphan (Task 5, Task 6).

---

# Phase 1: files core

### Task 1: The `files` crate: table, storage, put/insert/get/read/delete

**Files:**
- Create: `backend/crates/files/Cargo.toml`, `src/lib.rs`, `src/model.rs`, `src/repo.rs`, `src/service.rs`, `src/storage.rs`, `src/service/tests.rs`
- Create: `backend/migrations/NNNN_files_table.sql`
- Modify: `backend/Cargo.toml` (member `crates/files`, `files = { path = "crates/files" }`, `object_store = { version = "0.14", default-features = false, features = ["aws"] }`), `backend/crates/server/tests/architecture.rs` (`files` leaves `OTHER_SCHEMAS`, becomes an owned schema of the crate `files`), `backend/crates/server/tests/migrations.rs` if it lists tables, `AGENTS.md` section 4 and `ARCHITECTURE.md` (chain, module table row for `files.*`, the Known-limits line)

**Interfaces:**
- Produces (all `pub` from `files`):
  - `struct Storage` (`Clone`); `Storage::local(dir: &Path) -> Result<Storage, FilesError>`; `Storage::from_store(Arc<dyn object_store::ObjectStore>) -> Storage` (tests pass `InMemory`). `Storage::s3` arrives in Task 2.
  - `struct File { id: Uuid, uploaded_by: Uuid, name: String, mime: String, size: i64, sha256: String, created_at: DateTime<Utc> }`
  - `struct Stored { id: Uuid, size: i64, sha256: String, head: Vec<u8> }` (`head` = first 512 bytes)
  - `async fn put<B: AsRef<[u8]> + Into<PutPayload>>(storage: &Storage, id: Uuid, data: B, max_size: u64) -> Result<Stored, FilesError>`
  - `async fn insert(conn: &mut PgConnection, stored: &Stored, uploaded_by: Uuid, name: &str, mime: &str) -> Result<File, FilesError>`
  - `async fn get(conn: &mut PgConnection, id: Uuid) -> Result<Option<File>, FilesError>`
  - `async fn read(storage: &Storage, file: &File, range: Option<Range<u64>>) -> Result<GetResult, FilesError>` (`GetResult` re-exported)
  - `async fn delete(conn: &mut PgConnection, storage: &Storage, id: Uuid) -> Result<(), FilesError>` (row first, then object; a failed object delete is logged and ignored)
  - `async fn candidates(conn: &mut PgConnection, older_than: DateTime<Utc>) -> Result<Vec<Uuid>, FilesError>`
  - `enum FilesError { Empty, TooLarge, Storage(object_store::Error), Db(sqlx::Error) }` (`thiserror`)
- Table `files.files(id uuid pk, uploaded_by uuid not null, name text not null, mime text not null, size bigint not null check (size > 0), sha256 text not null, created_at timestamptz not null default now())`. No FK to other schemas.

- [ ] **Step 1: Write the failing tests** in `src/service/tests.rs` (`db_test`, `Storage::from_store(Arc::new(InMemory::new()))`): `put_then_insert_then_read_round_trips_bytes` (bytes equal, `size`, `sha256` of the known test vector `"abc"` = `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad`), `put_rejects_empty_data` (`FilesError::Empty`), `put_rejects_more_than_max_size_and_stores_nothing` (`TooLarge`, `InMemory` listing is empty), `read_with_range_returns_only_that_slice`, `delete_removes_row_and_object`, `candidates_lists_only_rows_older_than_cutoff` (insert two rows, back-date one with `update`).
- [ ] **Step 2: Run** `cargo test -p files` from `backend/`. Expected: FAIL, crate/functions not defined.
- [ ] **Step 3: Implement** the crate per the Interfaces. Storage key `"{id[0..2]}/{id[2..4]}/{id}"`. `sha256` as lower hex via `format!("{:x}", Sha256::digest(..))`. `repo.rs` holds every SQL statement (`query!`). Run the migration and `cargo sqlx prepare --workspace -- --all-targets`.
- [ ] **Step 4: Update** `architecture.rs`, `AGENTS.md` section 4 and `ARCHITECTURE.md` so `files` sits between `auth` and `events`. Run `cargo test -p files -p server --test architecture --test migrations`. Expected: PASS (the architecture test fails if the chain doc or the schema ownership is stale).
- [ ] **Step 5: Commit** `feat(files): the files crate with storage, put, insert, get, read, delete`

### Task 2: `[files]` settings and `Storage` in the state

**Files:**
- Modify: `backend/crates/config/src/settings.rs` (+ its tests.rs), `backend/crates/files/src/storage.rs`, `backend/crates/learning/src/state.rs`, `backend/crates/server/src/app.rs` (and wherever `LearningState` is built, including `tests/common/mod.rs` `test_settings`), `vumlo-cli` if it lists config keys, the config reference in `docs/` if one exists

**Interfaces:**
- Consumes: `files::Storage` (Task 1).
- Produces: `Settings.files: FilesSettings { storage: FileBackend (local|s3, default local) [public], path: Option<PathBuf> [private], max_size: u64 [public, default 52428800], s3: Option<S3Settings { endpoint: Option<String>, bucket: String, region: String }> [private], }`. S3 access key and secret are read from env `VUMLO_FILES_S3_ACCESS_KEY` / `VUMLO_FILES_S3_SECRET_KEY` (type `Secret`, never from the file). `Storage::s3(&S3Settings, access_key: &str, secret_key: &str) -> Result<Storage, FilesError>`. `LearningState.files: files::Storage` and `LearningState.max_file_bytes: u64`.

- [ ] **Step 1: Write the failing tests:** `defaults_to_local_storage_and_50_mib`, `s3_credentials_in_the_file_are_refused` (the existing secret-in-file rule), `relative_path_resolves_against_base_dir`, and a schema test that every new key is marked `public`/`private`/`secret` (follow the existing test that enforces this).
- [ ] **Step 2: Run** `cargo test -p vumlo-config`. Expected: FAIL.
- [ ] **Step 3: Implement** the settings and `Storage::s3` (`AmazonS3Builder`, custom endpoint allowed over https only, reuse `check_endpoint`); build `Storage` in `server` at startup (`expect("<reason>")` is allowed there); local default `<base_dir>/files`.
- [ ] **Step 4: Run** `cargo test -p vumlo-config -p server`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(config): [files] settings and the storage backend in the state`

### Task 3: Type sniffing and name cleaning

**Files:**
- Create: `backend/crates/files/src/mime.rs`, `src/mime/tests.rs`; Modify: `src/lib.rs`

**Interfaces:**
- Produces: `fn sniff(name: &str, head: &[u8], claimed: &str) -> String`; `fn is_inline(mime: &str) -> bool` (the six types of Global Constraints); `fn clean_name(raw: &str) -> String`.

- [ ] **Step 1: Write the failing tests:** `sniff_accepts_real_pdf_png_jpeg_gif_webp` (magic numbers `%PDF-`, `\x89PNG\r\n\x1a\n`, `\xFF\xD8\xFF`, `GIF8`, `RIFF....WEBP`), `sniff_demotes_a_fake_pdf_to_octet_stream` (name `a.pdf`, claimed `application/pdf`, bytes `hello`), `sniff_treats_svg_as_attachment_type` (`image/svg+xml` is not inline), `sniff_plain_text_requires_valid_utf8_without_nul`, `unknown_extension_keeps_a_guess_from_the_extension_table` (docx/pptx/xlsx/odt/zip/mp4/mp3), `clean_name_strips_path_parts_and_controls` (`../../etc/passwd` -> `passwd`, `a\\b/c.pdf` -> `c.pdf`), `clean_name_keeps_unicode`, `clean_name_truncates_to_200_chars_keeping_the_extension`, `clean_name_never_returns_empty` (falls back to `file`).
- [ ] **Step 2: Run** `cargo test -p files mime`. Expected: FAIL.
- [ ] **Step 3: Implement** with hand-written signature checks and one small extension-to-type table (docx, pptx, xlsx, odt, odp, ods, zip, mp3, mp4, txt, md, csv, json). The claimed type is only a hint for non-inline types.
- [ ] **Step 4: Run** `cargo test -p files`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(files): sniff the content type and clean file names`

### Task 4: Reference tables, ownership rules, and the usable-file check

**Files:**
- Create: `backend/migrations/NNNN_file_references.sql` (`content.item_files`, `assessment.submission_files`), `backend/crates/learning/src/files/{mod.rs, service.rs, repo.rs, model.rs, service/tests.rs}`
- Modify: `backend/crates/learning/src/lib.rs`, `content/repo.rs` and `assessment/repo.rs` (replace functions), `server/tests/architecture.rs` if table ownership lists need the new tables

**Interfaces:**
- Consumes: `files::{get, File}`; `content::open_item`, `content::open_assignment`, `assessment::authorize_view_submission`; actions `ViewContent`, `ManageContent`, `ViewOwnSubmission`, `ViewSubmissions`, `EditContent`, `SubmitWork`.
- Tables: `content.item_files(item_id uuid, file_id uuid, published bool, primary key (item_id, file_id, published))`; `assessment.submission_files(submission_id uuid, file_id uuid, primary key (submission_id, file_id))`. No cross-schema FKs; deleting an item or submission removes its rows (service deletes, or FK within the schema).
- Produces in `learning::files`:
  - `enum Owner { Item(Uuid), Assignment(Uuid) }` with `Owner::parse("item:<uuid>") -> Option<Owner>`
  - `async fn authorize_upload(conn: &mut PgConnection, actor: &Actor, owner: &Owner) -> Result<(), FileError>` (`EditContent` / `SubmitWork`; archived course -> `Conflict`)
  - `async fn open_for_read(conn: &mut PgConnection, actor: &Actor, file_id: Uuid) -> Result<files::File, FileError>`; every denial is `FileError::NotFound`
  - `async fn ensure_usable(conn: &mut PgConnection, actor: &Actor, owner: &UsageOwner, ids: &[Uuid]) -> Result<(), FileError>` where `enum UsageOwner { Item(Uuid), Submission(Uuid) }`; `FileError::Unusable { id: Uuid }` when a file does not exist, or belongs to another owner, or was uploaded by someone else and is referenced
  - `content::repo::replace_item_files(conn, item_id, published: bool, ids: &[Uuid])`, `assessment::repo::replace_submission_files(conn, submission_id, ids: &[Uuid])`
  - `enum FileError { NotFound, Conflict(&'static str), Unusable { id: Uuid }, TooLarge, Empty, Forbidden(PolicyError), Files(FilesError), Content(..), Db(sqlx::Error) }`

- [ ] **Step 1: Write the failing tests** (`db_test`, rows inserted directly into the reference tables to simulate an owner, since owners arrive in later phases): `uploader_reads_an_unreferenced_file`, `other_user_gets_not_found_for_an_unreferenced_file`, `student_gets_not_found_for_working_copy_file_of_a_published_item` (only the `published = false` row exists), `student_reads_published_row_of_a_visible_item`, `student_gets_not_found_when_item_not_yet_published_or_before_publish_at`, `staff_reads_working_copy_file`, `author_reads_own_submission_file`, `other_student_gets_not_found_for_a_submission_file`, `staff_gets_not_found_for_a_draft_submission_file`, `ensure_usable_accepts_own_unreferenced_upload`, `ensure_usable_accepts_file_already_on_this_owner`, `ensure_usable_rejects_file_of_another_owner_and_unknown_id`.
- [ ] **Step 2: Run** `cargo test -p learning files`. Expected: FAIL.
- [ ] **Step 3: Implement** the migration, repo functions (`query!`), and the service. A file is readable if **any** referencing row grants access. Prepare sqlx.
- [ ] **Step 4: Run** `cargo test -p learning files` and `cargo test -p server --test architecture`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(learning): file references and who may read or use a file`

### Task 5: Upload and download routes

**Files:**
- Create: `backend/crates/learning/src/files/routes.rs`, `backend/crates/api-types/src/files.rs` (`FileRefDto { id, name, mime, size }`, `utoipa`), `backend/crates/server/tests/files.rs`
- Modify: `learning/src/files/mod.rs` (`pub fn router()`), `backend/crates/server/src/app.rs`, `idempotency/mod.rs` and `conditional.rs` only if the bypass needs a path rule, `api-types/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 `put/insert/read`, Task 3 `sniff/is_inline/clean_name`, Task 4 `authorize_upload/open_for_read`, `LearningState.files/max_file_bytes`.
- Produces: `POST /api/v1/files?for=item:{id}|assignment:{id}&name=<filename>` -> `201 FileRefDto`; `GET /api/v1/files/{id}` -> `200`/`206` stream. `learning::files::router::<S>() -> OpenApiRouter<S>`.

- [ ] **Step 1: Write the failing API tests** in `server/tests/files.rs` (real server + `reqwest`, owner rows inserted by SQL): `upload_then_download_returns_identical_bytes`, `upload_is_authorized_before_the_body_is_read` (student posting `for=item:` gets `403`/`404` without the server consuming the body), `upload_zero_bytes_is_422_and_stores_nothing`, `upload_over_max_size_is_413_and_stores_nothing`, `upload_to_an_archived_course_is_409`, `upload_larger_than_2_mib_is_accepted` (proves the global limit is bypassed), `download_headers_for_pdf_png_text` (`inline`), `download_headers_for_svg_docx_and_fake_pdf` (`attachment`, fake PDF stored as `application/octet-stream`), `every_file_response_has_nosniff_and_sandbox_csp`, `content_disposition_survives_unicode_and_path_names` (RFC 8187 `filename*=UTF-8''...`), `etag_is_the_sha256_and_if_none_match_gives_304`, `range_request_returns_206_with_content_range`, `range_past_the_end_is_416`, `unreadable_file_is_404_not_403`.
- [ ] **Step 2: Run** `cargo test -p server --test files`. Expected: FAIL (404 on every route).
- [ ] **Step 3: Implement.** Handler order: parse `for`/`name`, `authorize_upload` (pooled connection), `axum::body::to_bytes(body, max_file_bytes)` (`413` on `LengthLimitError`), size 0 -> `422`, `files::put` before `begin`, `sniff` the mime from `Stored.head`, `begin`, one writing service call `register_upload` (authorizes again, `files::insert`), commit. Download: `open_for_read`, `files::read` with the parsed `Range`, `Body::from_stream(result.into_stream())`, headers per Global Constraints. In `app.rs`, `.merge()` the files router **after** the idempotency, conditional and 2 MiB `RequestBodyLimitLayer` layers and after the 30 s `TimeoutLayer`, wrapped in its own `RequestBodyLimitLayer(max_file_bytes)` and a 10-minute `TimeoutLayer`, but still inside the trace and `problem_details` layers. Map `FileError` to `ApiError` in one `From` impl (`TooLarge` -> 413, `Empty` -> 422 with `errors: [{field: "file", message}]`).
- [ ] **Step 4: Run** `cargo test -p server --test files --test idempotency --test conditional`. Expected: PASS (the last two prove the other routes are unchanged).
- [ ] **Step 5: Commit** `feat(server): upload and download routes for files`

### Task 6: Orphan sweep

**Files:**
- Create: `backend/crates/learning/src/files/sweep.rs`, tests in `service/tests.rs`
- Modify: `learning/src/files/mod.rs`, `backend/crates/server/src/main.rs` (spawn next to `events::run_worker` in both `worker` and `serve` unless `--no-worker`)

**Interfaces:**
- Produces: `async fn sweep(db: &PgPool, storage: &files::Storage, now: DateTime<Utc>) -> Result<u64, FileError>` (number deleted); `async fn run_sweeper(db: PgPool, storage: files::Storage)` (loops: `sweep(now)`, then sleeps one hour; logs file ids only).

- [ ] **Step 1: Write the failing tests** (fixed `now`, never sleep): `sweep_deletes_unreferenced_files_older_than_24h`, `sweep_keeps_unreferenced_files_younger_than_24h`, `sweep_keeps_a_file_referenced_by_an_item_working_or_published_row`, `sweep_keeps_a_file_referenced_by_a_submission`, `a_file_removed_from_every_reference_is_swept_after_24h` (Review Focus 3), `the_same_bytes_uploaded_twice_make_two_files_and_the_unused_one_is_swept` (Review Focus 5).
- [ ] **Step 2: Run** `cargo test -p learning sweep`. Expected: FAIL.
- [ ] **Step 3: Implement** with `files::candidates(now - 24h)`, drop those found in either reference table, `files::delete` each in its own short transaction.
- [ ] **Step 4: Run** `cargo test -p learning sweep` and `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(learning): sweep orphaned files after 24 hours`

### Task 7: Web: raw upload, `openFile`, and the mock

**Files:**
- Create: `web/src/lib/files.ts`, `web/src/mocks/handlers/files.ts`, `web/e2e/files.spec.ts`
- Modify: `web/src/lib/api/client.ts` (a raw-body request path next to `viaFetch`/`viaDesktop`), `web/src/mocks/handlers.ts`, `web/src/mocks/db.ts` (an in-memory file store), `web/openapi.json` + `pnpm gen:api`, `web/src/lib/i18n/*` (`files.open`, `files.download`, `files.tooLarge`, `files.typeNotAllowed`, `files.opened`, `files.savedToDownloads`)

**Interfaces:**
- Produces: `type FileRef = { id: string; name: string; mime: string; size: number }`; `uploadFile(file: File, target: { item: string } | { assignment: string }): Promise<FileRef>`; `openFile(ref: FileRef): Promise<void>`; `fileUrl(id: string): string` (`/api/v1/files/{id}`, used by `<img>`/`<iframe>` in the browser; in desktop `vumlo-file://{id}`); `readFileText(id: string, maxBytes: number): Promise<string>` (through `client.ts`). `openFile`: browser + `mime === "application/pdf"` -> `window.open(fileUrl, "_blank", "noopener")`; browser otherwise -> click a temporary `<a href download=name>`; desktop -> `invoke("open_file", { id })`.

- [ ] **Step 1: Write the failing Playwright specs** (`files.spec.ts`, mock mode): `upload_posts_raw_bytes_with_the_file_type` (intercept `POST /api/v1/files`, assert `content-type`, body length, `?for=`, `?name=` encoded), `open_pdf_opens_a_new_tab`, `open_docx_triggers_a_download_with_its_name`, `open_file_in_desktop_calls_the_open_file_command` (stub `window.__TAURI_INTERNALS__`), `upload_over_the_limit_shows_the_too_large_message` (mock replies 413).
- [ ] **Step 2: Run** the spec per the memory note. Expected: FAIL.
- [ ] **Step 3: Implement.** Do not `fetch` outside `client.ts`: add `requestRaw(method, path, body: Blob | ArrayBuffer, headers)` there with a desktop branch that calls `invoke("upload_file", { target, name, mime, bytes })` for `POST /files` only. The mock keeps bytes in memory and serves them with the same headers as the server. Add the missing endpoint entries to `API_GAPS.md` only if `gen:api` cannot yet type them.
- [ ] **Step 4: Run** `pnpm typecheck` and the spec. Expected: PASS.
- [ ] **Step 5: Commit** `feat(web): upload and open files`

### Task 8: Desktop: `upload_file`, `open_file`, `vumlo-file://`

**Files:**
- Create: `desktop/src/files.rs`, `desktop/src/files/tests.rs`
- Modify: `desktop/src/lib.rs` (register commands and the URI scheme), `desktop/src/api.rs`, `desktop/src/core.rs` (a binary request path next to the JSON one), `desktop/tauri.conf.json` (CSP `img-src`/`frame-src` gain `vumlo-file:`), `docs/decisions/done.md` (amend: the two commands and the scheme)

**Interfaces:**
- Produces: `#[tauri::command] async fn upload_file(core, target: String, name: String, mime: String, bytes: Vec<u8>) -> Result<FileRef, String>`; `#[tauri::command] async fn open_file(app, core, id: String) -> Result<OpenOutcome, String>` where `enum OpenOutcome { Opened, Revealed, SavedOnly }`; `fn unique_path(dir: &Path, name: &str) -> PathBuf` (`name.pdf`, `name (1).pdf`, ...); `fn safe_name(raw: &str) -> String`; `fn opens_directly(name: &str) -> bool` (allowlist: pdf, doc, docx, odt, rtf, txt, md, ppt, pptx, odp, xls, xlsx, ods, csv, png, jpg, jpeg, gif, webp, mp3, wav, mp4, webm, mov, zip, 7z, tar, gz; case-insensitive; **allowlist, so unknown is false**).

- [ ] **Step 1: Write the failing tests** in `files/tests.rs`: `allowlist_opens_documents_images_media_archives`, `executables_and_scripts_are_never_opened` (`exe bat cmd com sh app lnk scr msi jar desktop ps1 vbs js` and `report.pdf.exe`, `.PDF` upper case is allowed), `unknown_extension_is_not_opened`, `unique_path_adds_a_counter_before_the_extension`, `unique_path_handles_a_name_without_extension`, `safe_name_strips_separators_and_traversal`, `vumlo_file_uri_accepts_only_uuid_paths`.
- [ ] **Step 2: Run** `cargo test` in `desktop/`. Expected: FAIL.
- [ ] **Step 3: Implement.** `open_file`: request `GET /api/v1/files/{id}` through `Core` (token stays in Rust), write to the system Downloads directory with `unique_path`, then `opener::open_path` if `opens_directly`, else `reveal_item_in_dir`; an opener error returns `SavedOnly` (the file stays) and the web shows the `files.savedToDownloads` toast. The URI scheme handler accepts only `vumlo-file://<uuid>`, fetches with the token and returns the body with the server's `Content-Type`. No change to `capabilities/default.json`.
- [ ] **Step 4: Run** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` in `desktop/`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(desktop): upload and open files, vumlo-file scheme`

---

# Phase 2: lesson material

### Task 9: Item kind `file`

**Files:**
- Create: `backend/migrations/NNNN_item_kind_file.sql` (extend the CHECK on `content.items.kind`)
- Modify: `backend/crates/learning/src/content/{model.rs, service.rs, repo.rs, routes.rs}`, `backend/crates/api-types/src/lib.rs` (create-item request gains `kind`), `backend/crates/server/tests/content.rs`, `docs/decisions/in-progress.md` (note: built, kind field), `web` openapi + `gen:api`

**Interfaces:**
- Consumes: Task 4 `ensure_usable`, `replace_item_files`.
- Produces: `ItemKind::File`; body `{"file_id": uuid, "description"?: document}`; create-item request field `kind: "text" | "assignment" | "file"` (optional; inferred when absent so old clients keep working); `create_item`, `update_item` and `publish_item` rewrite `content.item_files` (working row on every body write, published row on publish; a file removed from the body leaves the table).

- [ ] **Step 1: Write the failing tests** in `server/tests/content.rs`: `teacher_creates_a_file_item_with_an_uploaded_file`, `file_item_with_someone_elses_file_is_422`, `file_item_description_must_be_a_valid_full_profile_document`, `student_cannot_open_a_draft_file_item_or_its_file` (item `404`, file `404`), `publishing_makes_the_file_readable_by_students_only_after_publish_at`, `replacing_the_file_removes_the_old_reference` (old id gone from `item_files`, `GET /files/{old}` is `404` for students, still readable by staff only if some row names it), `kind_absent_is_still_inferred_for_old_clients`.
- [ ] **Step 2: Run** `cargo test -p server --test content`. Expected: FAIL.
- [ ] **Step 3: Implement** migration, `ItemKind::File`, validation of the body, `ensure_usable` before the write, and the reference rewrites in the same transaction. Prepare sqlx, regenerate `web/openapi.json`/schema.
- [ ] **Step 4: Run** the Phase 1 definition-of-done backend commands. Expected: all pass.
- [ ] **Step 5: Commit** `feat(content): the file item kind`

### Task 10: Web: file item in the course and in authoring

**Files:**
- Create: `web/src/components/item/FilePage.tsx`
- Modify: `web/src/components/item/ItemView.tsx`, `web/src/components/ItemTile.tsx` (a `"file"` tile kind with its own icon and `var(--token)` colour), `web/src/components/course/StudentContent.tsx`, `web/src/components/author/*` (create-item menu "File", upload field, description editor), `web/src/mocks/handlers/*.ts`, `web/src/lib/i18n/*`, `web/e2e/routes.ts`, `web/e2e/files.spec.ts`

**Interfaces:**
- Consumes: `uploadFile`, `openFile`, `FileRef` (Task 7); the extended create-item request (Task 9).

- [ ] **Step 1: Write the failing specs:** `teacher_adds_a_pdf_as_lesson_material`, `student_sees_the_file_row_and_opens_it`, `file_item_page_shows_description_and_open_card`, `replacing_the_file_updates_the_card`.
- [ ] **Step 2: Run** the specs. Expected: FAIL.
- [ ] **Step 3: Implement.** Student list rows use `ItemTile` kind `file`; `FilePage` renders the description with `BlockRenderer` and an "Open" card with name and size, calling `openFile`.
- [ ] **Step 4: Run** `pnpm typecheck && pnpm build`, the specs, and open the screens under `pnpm dev:mock` in light and dark. Expected: PASS, no console errors.
- [ ] **Step 5: Commit** `feat(web): file items as lesson material`

---

# Phase 3: file assignment

### Task 11: Assignment type `file`

**Files:**
- Create: `backend/crates/learning/src/content/types/file.rs`, `types/file/tests.rs`
- Modify: `backend/crates/learning/src/content/{model.rs (BuiltinType::File and every match arm, error text at the line that lists the types), types/mod.rs}`, `api-types` doc strings that list the types, `content/routes.rs` doc strings, `docs/decisions/done.md`, `0008` if it lists built types

**Interfaces:**
- Produces: `BuiltinType::File`; config `{ "max_files": 1..=20 (default 5), "accept"?: [".pdf", ...] }` (lowercase, leading dot, at most 30 entries); content `{ "files": [uuid], "comment"?: document }`; `validate_submission(Stage::Draft)`: `files.len() <= max_files`, duplicates refused, every file's cleaned extension in `accept` when set (case-insensitive), `comment` valid in the `student` profile; `Stage::Submit` additionally at least one file; `auto_grade` none; `claims` lists the file ids.

- [ ] **Step 1: Write the failing tests** in `types/file/tests.rs`: `config_defaults_max_files_to_5`, `config_rejects_max_files_0_and_21`, `config_rejects_accept_without_a_dot_or_uppercase`, `draft_over_max_files_is_rejected`, `draft_with_a_file_outside_accept_is_rejected`, `accept_is_case_insensitive_for_the_file_name`, `duplicate_file_ids_are_rejected`, `submit_needs_at_least_one_file`, `draft_may_be_empty`, `comment_uses_the_student_profile`.
- [ ] **Step 2: Run** `cargo test -p learning content::types::file`. Expected: FAIL.
- [ ] **Step 3: Implement** following `types/quiz.rs`/`form.rs` (the `AssignmentType` trait at `model.rs`). The accepted-extension check needs each file's name: `validate_submission` stays pure by receiving names through the existing `Claims`/context path, or the assessment service resolves names with `files::get` and passes them in; choose the existing pattern that quiz uses for external data and ledger the choice.
- [ ] **Step 4: Run** `cargo test -p learning`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(content): the file assignment type`

### Task 12: Submissions and their files

**Files:**
- Modify: `backend/crates/learning/src/assessment/{service.rs, repo.rs, routes.rs}`, `server/tests/submissions.rs`, `docs/decisions/0011` or `0035` only if they state the content shape

**Interfaces:**
- Consumes: Tasks 4 and 11.
- Produces: `save_draft` and `submit` call `ensure_usable(UsageOwner::Submission(id), ..)` and `replace_submission_files` in the same transaction; uploads for `for=assignment:{id}` are refused with `409` when the submission window is closed or the course archived; a draft that drops a file removes its row.

- [ ] **Step 1: Write the failing tests** in `server/tests/submissions.rs`: `student_uploads_and_saves_a_draft_with_files`, `student_cannot_save_a_draft_naming_another_students_file` (`422`), `submit_stores_the_files_and_staff_can_download_them`, `staff_cannot_download_a_file_of_a_draft_submission`, `another_student_gets_404_for_the_file`, `upload_after_the_window_closed_is_409_and_stores_nothing`, `removing_a_file_from_the_draft_drops_its_reference`, `resubmission_attempt_two_keeps_attempt_one_files_readable_by_staff`, `grades_and_draft_grade_fields_never_reach_the_student_file_response`.
- [ ] **Step 2: Run** `cargo test -p server --test submissions`. Expected: FAIL.
- [ ] **Step 3: Implement.** Attempts are rows (`submit` turns the draft row into attempt n): the reference rows follow the row id, so attempt files stay readable after the next attempt starts a new draft row.
- [ ] **Step 4: Run** the full backend definition of done (with and without `--features dummy`). Expected: all pass.
- [ ] **Step 5: Commit** `feat(assessment): submissions carry files`

### Task 13: Web: student file submission and assignment settings

**Files:**
- Create: `web/src/components/item/FileAssignmentPage.tsx` (+ CSS module)
- Modify: `web/src/components/item/ItemView.tsx` (route the `file` type), `web/src/components/author/AssignmentSettings.tsx` (`max_files`, `accept`), mocks, i18n, `web/e2e/files.spec.ts`

**Interfaces:**
- Consumes: `uploadFile`, `openFile`, `FileRef`; existing draft hooks and autosave (`lib/autosave.ts`, `useSubmitDraft`).

- [ ] **Step 1: Write the failing specs:** `student_drops_two_files_and_submits`, `student_removes_a_file_before_submitting`, `student_sees_an_error_for_a_type_outside_accept`, `submit_is_disabled_with_no_files`, `teacher_sets_max_files_and_accept_in_settings`, `submitted_attempts_list_their_files_with_open_buttons`.
- [ ] **Step 2: Run** the specs. Expected: FAIL.
- [ ] **Step 3: Implement.** Drop zone with `<input type="file" multiple accept=...>`; list with remove; comment in `BlockEditor` (student profile); autosave through the existing draft path; uploading state per file; queued/offline writes are not used for uploads (a screen that needs a connection says so with `useOffline()`).
- [ ] **Step 4: Run** `pnpm typecheck && pnpm build`, the specs, and both themes. Expected: PASS.
- [ ] **Step 5: Commit** `feat(web): file assignment page and settings`

### Task 14: Web: grade-mode preview

**Files:**
- Create: `web/src/components/grading/FilePreview.tsx` (+ CSS module)
- Modify: `web/src/components/grading/SubmissionView.tsx` (the `Body` switch on `item.assignment.type`), `web/e2e/grading.spec.ts`, mocks, i18n

**Interfaces:**
- Consumes: `FileRef`, `fileUrl`, `readFileText`, `openFile`, `isDesktop`.

- [ ] **Step 1: Write the failing specs:** `teacher_previews_a_pdf_inline` (an `<iframe src=/api/v1/files/...>`), `teacher_previews_an_image`, `teacher_previews_text_up_to_1_mib_and_sees_a_truncation_note_beyond`, `teacher_sees_an_open_card_for_a_pptx`, `desktop_linux_shows_an_open_card_for_a_pdf` (stub desktop + platform), `switching_attempt_tabs_switches_the_file_list`, `comment_is_shown_under_the_files`.
- [ ] **Step 2: Run** the specs. Expected: FAIL.
- [ ] **Step 3: Implement.** Selected file state is local; inline set is exactly the server's inline set (PDF, PNG, JPEG, GIF, WebP, text/plain); others show the card; text is read with `readFileText(id, 1 MiB)`; never `dangerouslySetInnerHTML`.
- [ ] **Step 4: Run** `pnpm typecheck && pnpm build`, `grading.spec.ts`, `files.spec.ts`, both themes. Expected: PASS.
- [ ] **Step 5: Commit** `feat(web): preview submitted files while grading`

---

# Phase 4: editor

### Task 15: `image` and `file` blocks in documents

**Files:**
- Modify: `backend/crates/api-types/src/document.rs` (`Block::Image { file_id, alt, caption }`, `Block::File { file_id }`; `Feature::Image`, `Feature::File`), `backend/crates/learning/src/content/document/{parse.rs, profile.rs, text.rs, mod.rs}`, `content/service.rs` and `assessment/service.rs` (collect ids from the document for the reference rewrites), `server/tests/content.rs`, `docs/decisions/done.md`, `0027-*.md`, `ARCHITECTURE.md` (Known limits line about empty documents), `web` openapi + `gen:api`

**Interfaces:**
- Consumes: `ensure_usable`, `replace_item_files`, `replace_submission_files`.
- Produces: `fn file_ids(document: &Document) -> Vec<Uuid>` (`document/mod.rs`); the parser stops rejecting `"image"` and keeps rejecting `"html"` and a non-empty `attachments`; `image` allowed in every profile, `file` in `full` and `student`; `is_blank` treats an `image` or `file` block as content; item/submission writes call `ensure_usable` for `file_ids` then rewrite references (the `file` item kind's `file_id` and the blocks of its description both count).

- [ ] **Step 1: Write the failing tests:** unit in `document/tests.rs`: `image_block_parses_in_every_profile`, `file_block_is_refused_in_question_and_option_profiles`, `attachments_must_still_be_empty`, `html_block_is_still_refused`, `a_document_of_only_an_image_is_not_blank`, `file_ids_collects_images_and_files_in_order_without_duplicates`; API in `server/tests/content.rs`: `item_body_with_an_image_of_an_uploaded_file_is_saved_and_students_read_it_after_publish`, `item_body_naming_someone_elses_file_is_422_with_the_block_path`, `removing_the_image_from_the_body_drops_the_reference`, `student_text_answer_with_a_file_block_saves_and_staff_can_read_it_after_submit`, `quiz_question_text_accepts_an_image_but_not_a_file_block`.
- [ ] **Step 2: Run** `cargo test -p learning document` and `cargo test -p server --test content`. Expected: FAIL.
- [ ] **Step 3: Implement** the blocks, profile table, parser path-errors (`blocks[3].file_id`), blank rule, and reference rewrites in the same transactions as the writes. The 2 MiB document limit counts JSON only. Regenerate openapi and the schema.
- [ ] **Step 4: Run** the full backend definition of done (with and without `--features dummy`). Expected: all pass.
- [ ] **Step 5: Commit** `feat(content): image and file blocks in documents`

### Task 16: Web: editor upload, `image` and `file` blocks

**Files:**
- Modify: `web/src/components/doc/model.ts` (`InsertKind`, `makeBlock`, `blockMenu`), `web/src/components/doc/BlockEditor.tsx` (slash entries, `onPaste` for files, drop handling, placeholder state), `web/src/components/doc/BlockRenderer.tsx`, `web/src/components/doc/Doc.module.css`, i18n, mocks, `web/e2e/editor.spec.ts`; Create: `web/src/components/doc/FileCard.tsx`

**Interfaces:**
- Consumes: `uploadFile`, `openFile`, `FileRef`; `Feature` now includes `image` and `file`; the editor needs an upload target (`{ item } | { assignment }`) as a new optional prop `uploadTarget`; without it the image/file entries are hidden.

- [ ] **Step 1: Write the failing specs** in `editor.spec.ts`: `slash_menu_offers_image_and_file_only_with_an_upload_target`, `pasting_an_image_uploads_and_inserts_an_image_block`, `dropping_a_pdf_inserts_a_file_card`, `a_placeholder_shows_while_uploading_and_is_replaced`, `a_failed_upload_removes_the_block_and_shows_a_toast`, `renderer_shows_an_image_and_a_file_card_that_opens`, `student_editor_has_image_and_file_but_question_editor_has_no_file`.
- [ ] **Step 2: Run** the specs. Expected: FAIL.
- [ ] **Step 3: Implement.** `onPaste` currently turns the clipboard into plain text only: files in `clipboardData.files` go to the upload path first, text keeps the old path. `<img src>` uses `fileUrl(id)`; in desktop the `vumlo-file://` URL. Callers that edit an item pass `{ item: id }`, the student text assignment passes `{ assignment: id }`.
- [ ] **Step 4: Run** `pnpm typecheck && pnpm build`, `editor.spec.ts`, `files.spec.ts`, `smoke.spec.ts` (about 11 minutes; do not edit files while it runs), both themes. Expected: PASS, no console errors.
- [ ] **Step 5: Commit** `feat(web): upload images and files in the editor`

### Task 17: Documentation and final checks

**Files:**
- Modify: `docs/decisions/done.md` (confirm the `object_store` features used), `docs/decisions/0023`, `0024`, `0025`, `0027`, `0061` (amend lines pointing at ADR-0069 where Tasks 8, 9, 11, 15 did not already), `README.md` or the deployment doc (reverse proxy body size, back up the files directory or bucket separately from `db backup`), `web/API_GAPS.md` (remove entries the real endpoints now cover)

- [ ] **Step 1: Check** each ADR named in ADR-0069's `Amends:` line has an "Amended by ADR-0069" note, and each sentence the new behaviour makes false ("must be empty for now" for images, "needs the files module") is fixed.
- [ ] **Step 2: Run** every definition-of-done command for backend, web and desktop (Global Constraints). Expected: all pass.
- [ ] **Step 3: Manual check** (spec section 13): mock mode and a real server in local Chromium: upload in the editor, a file item, a file submission, then the grading preview; on desktop open a PDF and a DOCX and confirm a `.sh` file is only revealed in its folder.
- [ ] **Step 4: Commit** `docs(files): amend the ADRs and document deployment of file storage`
