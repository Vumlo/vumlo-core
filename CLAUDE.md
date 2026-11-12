# CLAUDE.md

Rules for working in this repository. Section numbers are cited from code comments (as "AGENTS §N") and
from `RELEASING.md`, so they stay as they are.

## 1. Sources of truth

- `ARCHITECTURE.md` is the current design; `docs/PLAN.md` is its short form with the plan.
- `docs/decisions/{done,in-progress,planned}.md` list the accepted decisions, one line each. They override
  everything else, including this file's examples. If a task conflicts with one, ask before diverging. The full
  text of an ADR: `git show 8052baf:docs/decisions/<number>-*.md`.
- Dependencies are the crates and features listed in ADR-0014. A new crate, version bump or feature flag needs an ADR.

## 4. Architecture (ADR-0001, ADR-0016, ADR-0065)

Each layer is a crate, and a crate depends only on the ones below it. Cargo enforces the direction.

    server, vumlo-cli
      -> plugins    plugin installs, event delivery to plugins, the plugin API (ADR-0066)
      -> modules/*  optional crates behind Cargo features, built on module-api (ADR-0066)
      -> learning   teaching, content, assessment, gradebook, and the flows that need several of them
      -> org        terms, groups, subgroups, the installation record
      -> auth       users, sessions, sign-in, account value types, the text of its mails (it queues them in mail)
      -> mail       outgoing mail: the queue, the worker, opt-outs, the transport; cannot read auth (ADR-0084)
      -> files      stored bytes and their description: the files table and the storage backend (ADR-0069)
      -> events     audit writes, the outbox and its worker
      -> access     Actor and authorize
      -> common     Role, ApiError, pagination, text_enum!

`db` (migrations, the installation record, the CLI's audit row) is a crate beside the chain.
`test-support` is the test database harness and a dev-dependency only.

The configuration schema is `backend/crates/config` (`vumlo-config`), shared with `vumlo-cli`: a new
key must be marked `public`, `private` or `secret` (ADR-0054), and a secret is never read from the
file. The administrator CLI is `backend/crates/vumlo-cli`; a CLI command that changes the database
writes an audit row (source `cli`). An optional module is its own crate under `backend/crates/modules/`
behind a Cargo feature, and `server` and `vumlo-cli` name it in exactly one place each.

    <crate>/src/<module>/        (a crate with one module has it at the root: lib.rs)
      mod.rs     the ONLY public surface: pub fn router(), pub service functions, pub types
      model.rs   domain types and newtypes (public)
      routes.rs  private: extractors, DTO ↔ domain mapping, utoipa annotations
      service.rs private: business rules; exposed only through mod.rs re-exports
      repo.rs    private: SQL only

- `routes`, `service` and `repo` are declared without `pub`. Other modules use only what `mod.rs` exposes.
- Each module owns its Postgres schema and writes only to it. To change another module's data, call that
  module's public function and pass the connection.
- **Reads across layers:** a layer reads the tables of its own layer, and those of a lower layer **only through a view
  that layer declared** (`auth.people`, `org.term_list`, `org.group_list`, `org.subgroup_list`, `events.audit_entries`,
  and `teaching.effective_members` for the roster). It never names the schema of a higher layer. Inside `learning`
  the four modules may read one another (the gradebook is a read model over content and assessment). Declaring a view
  is a migration and a line in `architecture.rs`.
- **What needs the data of several layers is a flow in `learning`**, one service function with one transaction
  (`learning::people::update_user`), never a call from a lower crate to a higher one. `auth::apply_user_update` is
  called only from `learning`.
- SQL lives only in `repo.rs`. HTTP types (`axum`, DTOs) never appear in `service.rs` or `repo.rs`.
  Exception: the block document types of ADR-0025 and ADR-0027 live in `api-types` and `content` uses them directly.
- A large file becomes a folder (`routes/courses.rs`, `routes/groups.rs`); the module's public surface does not change.
- `Actor::new` and `Actor::role` are private to `access`; a test builds an actor with `Actor::for_tests` (feature `testing`).
  A crate enables `testing` of another only under `[dev-dependencies]`.

### Transactions

- Service and repo functions take `&mut PgConnection`. They never call `begin` or `commit`.
- **Writes:** the route handler (or CLI command) opens the transaction, calls exactly one writing service function, and commits.
  Reads and slow preparation (looking up a user, verifying or hashing a password) are separate service calls made **before** `begin`.
  If an action spans modules, the owning module's service calls the others and passes the same connection
  (e.g. `gradebook::publish_grades` calls `events::emit`).
- **Reads:** use a pooled connection without a transaction. Use a read-only transaction only when several
  queries must see one consistent snapshot.
- **Nothing slow inside a transaction:** no hashing, no HTTP calls; write a file to storage first, then insert
  its metadata in the transaction (orphans are cleaned by a job).

## 5. Authorization

- Every service function acting for a user takes an `Actor` and authorizes (`access::authorize(&actor, Action)`)
  before it changes or reveals anything.
  The resource a check needs travels inside the `Action` variant: when the action needs the resource, load it first,
  then authorize, and report a denied view as "not found".
  A handler may also call it early, before costly work such as hashing, but the service still authorizes first.
- No role checks anywhere except `access`. Deny by default.
- Every action by an `admin` inside a course that changes data writes to `events.audit_log` in the same transaction; reads are not audited.

## 6. Visibility invariants (each one must have a test)

- Students and any non-staff response **never** contain `draft_*` grade fields.
  Student-facing repo functions do not select those columns. In `gradebook` that is
  `my-grades`, served by one repo function that selects `published_*` only.
- Students never see items unless course, module and item are published and `publish_at <= now()`.
- Staff never see submissions with `status = 'draft'`.
- A student never receives the answer key of a `quiz`: not in `config` (`correct`, `answer`, `tolerance`,
  `accepted`, `explanation`, `partial_credit`, `grading_notes` are removed by `student_config`), and not
  in their own attempts, where `auto_scores` is staff-only. The key and the explanations reach a
  student only through `GET /assignments/{id}/result`, after their grade is published and only if
  `show_correct` is on (ADR-0050).
- A student sees who holds an exclusive option of a `form` only as `taken`, never by name (ADR-0051).
- A student never receives the body, the questions or the files of an item that an access rule keeps
  closed, and cannot save, submit or upload to it; a student never receives the rule itself, only
  where they stand (ADR-0092).

## 7. Events

- A state change and its event are written in the same transaction via `events::emit(&mut conn, ...)`.
- Saving drafts (grades, content, submissions) emits no events.
- Payloads contain ids only: no grades, content or personal data.

## 8. Database

- Use `sqlx::query!` / `query_as!` (compile-time checked). `QueryBuilder` only for dynamic filters, with bound
  parameters. Never build SQL with `format!`.
- Migrations are append-only: never edit a committed one, write a new one.
- Every table is schema-qualified (`teaching.courses`). Enums are `text` + `CHECK`, not Postgres `ENUM`.
  Scores are `NUMERIC`, ids are UUID v7, timestamps are `timestamptz`.
- After changing any query, run `cargo sqlx prepare --workspace -- --all-targets` and commit `backend/.sqlx/`.
  `--all-targets` makes `.sqlx/` cover the tests too, so `query!` is fine in tests. A runtime query is for SQL
  that is not ours to check (a system view) or whose text is a parameter.

## 9. API

- Everything under `/api/v1`; changes are additive only.
- DTOs live in `backend/crates/api-types`, are validated with `garde`, and are annotated for `utoipa`.
- Every error response, including framework-generated ones (405/408/413, extractor rejections), is RFC 9457
  Problem Details from the one `IntoResponse` impl. Validation failures are `422` with `errors: [{field, message}]` (ADR-0018).
- Lists use cursor pagination: `?cursor=&limit=` → `{items, next_cursor}` (ADR-0018). `Idempotency-Key` on `POST`
  and `ETag` on `GET` are layers (ADR-0058, ADR-0059): handlers do nothing for them.
- A body that cannot be read is `422`, a wrong media type `415`, a database that is busy or down `503` with
  `Retry-After` (ADR-0039). Any other database error is a logged `500`.

## 10. Errors

- Each module has a `thiserror` error enum; `anyhow` only in `main`, CLI code and tests.
- No `unwrap()`, `expect()` or `panic!` outside tests, except `expect("<reason>")` during startup in `main`.

## 11. Security

- Passwords: `argon2`, inside `tokio::task::spawn_blocking`, before opening a transaction.
- Session tokens, device codes and webhook secrets are stored hashed or encrypted. Compare secrets in constant time.
- Responses that carry a credential (a token, a temporary password) send `Cache-Control: no-store`.
- Types that hold secrets (`Password`, `SessionToken`, DTOs with passwords) redact them in `Debug`.
- Never log passwords, tokens, cookies or email addresses; log user ids.

## 12. Tests

- Every change comes with tests that check behavior. Do not delete or weaken a test to make it pass.
- Unit tests never sit inside a source file: `foo.rs` declares `#[cfg(test)] mod tests;` and the tests are in
  `foo/tests.rs` (for `mod.rs` and `lib.rs`: `tests.rs` beside it). The release archive leaves out every
  `tests.rs` and the `test-support` crate (`RELEASING.md`), and an inline test would ship.
- DB tests are `#[tokio::test]` functions that run in `db_test(async |pool| { … })` (ADR-0036); never `#[sqlx::test]`.
  API tests start the real server and call it with `reqwest`. Shared helpers (server spawn, seeding, login) live in
  `backend/crates/server/tests/common/`.
- A test never holds a pooled connection across a request to the server, and never waits for a lock with a fixed
  sleep: it uses `test_support::wait_until_blocked`.

## 13. Old code

- Old code is deleted, git keeps history: no `legacy/` or `*_v2` copies, no compatibility layers, shims or
  feature flags for "old behavior".

## 14. Definition of done

All of these pass, run from `backend/`:

    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    cargo test --workspace -- --test-threads=16
    cargo sqlx prepare --workspace --check -- --all-targets
    SQLX_OFFLINE=true cargo check --workspace --all-targets

and the same with `--features dummy` (cargo test, clippy and the offline check); `cargo sqlx prepare`
runs once with `--all-features`. From `web/`, after `vumlo-cli brand apply`: `npm run typecheck` and
`npm run build`.
From `desktop/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` and `npm test`
(its Linux libraries are listed in `docs/desktop.md`). It has its own Cargo workspace; no crate there is part of
ADR-0014, its dependencies are those of ADR-0056, ADR-0061 and ADR-0062.

## 15. Git

- Work on a branch, never on `main`; push only when asked.
- Small commits in Conventional Commits format (`feat(teaching): ...`).
