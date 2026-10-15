# AGENTS.md

Rules for any coding agent working in this repository. They are not suggestions.
If a rule blocks the task, stop and say so. Do not work around it.

## 1. Sources of truth

- `ARCHITECTURE.md`: the current design.
- `docs/decisions/`: accepted decisions (ADRs). They override everything else, including this file's examples.
- If a task conflicts with an ADR, **stop and ask**. Never silently diverge.
- If a task needs a decision no ADR covers (new table shape, new dependency, new convention),
  **stop and propose 2–3 options with trade-offs**. Do not pick one yourself.

## 2. Scope

- Do exactly the task. No drive-by refactors, renames or "improvements".
- Problems found outside the task go into your final message, not into the code.
- Do not create files the task does not need.

## 3. Dependencies

- Only crates listed in ADR-0014, with the features listed there.
- A new crate, a version bump or a new feature flag means: stop and propose an ADR.
- Features required to implement a use already described in ADR-0014 are allowed without a new ADR, but must be listed in the plan.

## 4. Architecture (ADR-0001, ADR-0015, ADR-0016)

Modules: `identity`, `org`, `teaching`, `content`, `assessment`, `gradebook`, `files`, `events`, `policy`.

    crates/server/src/
      main.rs      CLI entry: serve | migrate | create-admin | import-users
      app.rs       AppState, router assembly (nests each module's router())
      error.rs     shared API error → Problem Details
      actor.rs     Actor: who performs the action
      <module>/
        mod.rs     the ONLY public surface: pub fn router(), pub service functions, pub types
        model.rs   domain types and newtypes (public)
        routes.rs  private: extractors, DTO ↔ domain mapping, utoipa annotations
        service.rs private: business rules; exposed only through mod.rs re-exports
        repo.rs    private: SQL only

- `routes`, `service` and `repo` are declared without `pub`. Other modules may use only what `mod.rs` exposes.
- Each module owns its Postgres schema. **A module writes only to its own schema.**
  To change another module's data, call that module's public function and pass the connection.
- Cross-module JOINs are allowed **only in read-model queries** (e.g. the gradebook view in `gradebook`), never for writes.
- SQL lives only in `repo.rs`. HTTP types (`axum`, DTOs) never appear in `service.rs` or `repo.rs`.
- A large file becomes a folder (`routes/courses.rs`, `routes/groups.rs`). The module's public surface does not change.

### Transactions

- Service and repo functions take `&mut PgConnection`. They **never** call `begin` or `commit`.
- **Writes:** the route handler (or CLI command) opens the transaction, calls **exactly one** service function, and commits.
  If an action spans modules, the owning module's service calls the others and passes the same connection
  (e.g. `gradebook::publish_grades` calls `events::emit`).
- **Reads:** use a pooled connection without a transaction. Use a read-only transaction only when several
  queries must see one consistent snapshot.
- **Nothing slow inside a transaction:** hash passwords before `begin`; never make HTTP calls inside a transaction;
  write a file to storage first, then insert its metadata in the transaction (orphans are cleaned by a job).

## 5. Authorization

- Every service function acting for a user takes an `Actor` and calls `policy::authorize(actor, action, resource)` before anything else.
- No role checks anywhere except `policy`. Deny by default.
- Every action by an `admin` inside a course writes to `events.audit_log` in the same transaction.

## 6. Visibility invariants (each one must have a test)

- Students and any non-staff response **never** contain `draft_*` grade fields.
  Student-facing repo functions do not select those columns.
- Students never see items unless course, module and item are published and `publish_at <= now()`.
- Staff never see submissions with `status = 'draft'`.

## 7. Events

- A state change and its event are written in the **same transaction** via `events::emit(&mut conn, ...)`.
- Saving drafts (grades, content, submissions) emits **no** events.
- Payloads contain ids only. Never put grades, content or personal data in a payload.

## 8. Database

- Use `sqlx::query!` / `query_as!` (compile-time checked). `QueryBuilder` only for dynamic filters,
  always with bound parameters. **Never build SQL with `format!`.**
- Migrations are append-only. Never edit a migration that is already committed; write a new one.
- Every table is schema-qualified (`teaching.courses`).
- Enums are `text` + `CHECK` constraint, not Postgres `ENUM`.
- Scores are `NUMERIC`, ids are UUID v7, timestamps are `timestamptz`.
- After changing any query, run `cargo sqlx prepare --workspace` and commit `.sqlx/`.

## 9. API

- Everything under `/api/v1`. Changes are additive only.
- Request/response DTOs live in `crates/api-types`, are validated with `garde`, and are annotated for `utoipa`.
- Errors are RFC 9457 Problem Details, produced by one `IntoResponse` impl.
- All error responses, including framework-generated ones (405/408/413, extractor rejections), are Problem Details.
- Lists use cursor pagination. Non-idempotent POSTs honor `Idempotency-Key`.

## 10. Errors and panics

- Each module has a `thiserror` error enum. `anyhow` only in `main`, CLI code and tests.
- No `unwrap()`, `expect()` or `panic!` outside tests, except `expect("<reason>")` during startup in `main`.

## 11. Security

- Passwords: `argon2`, always inside `tokio::task::spawn_blocking`, and before opening a transaction.
- Session tokens, device codes and webhook secrets are stored hashed or encrypted, never in plain text.
- Compare secrets in constant time.
- Never log passwords, tokens, cookies or email addresses. Log user ids.

## 12. Tests

- Every change comes with tests that check behavior, not just "it compiles".
- DB tests use `#[sqlx::test]`. API tests start the real server and call it with `reqwest`.
- **Never delete, skip or weaken a test to make it pass.** If a test looks wrong, say so and stop.

## 13. Forbidden

- Report / summary / status `.md` files of any kind.
- `legacy/`, `old/`, `archive/`, `backup/` directories or `*_v2` copies. Old code is deleted, git keeps history.
- Compatibility layers, shims, feature flags for "old behavior".
- Commented-out code, `TODO` without an issue reference, fake or stub implementations presented as done.
- `#[allow(...)]` to silence clippy without a comment explaining why.

## 14. Definition of done

All of these pass:

    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    cargo sqlx prepare --workspace --check

## 15. Git

- Work on a branch, never on `main`. Do not push or force-push unless asked.
- Small commits in Conventional Commits format (`feat(teaching): ...`).

## 16. Final message

End every task with a short message (not a file):
1. What changed and why.
2. Any decision you had to make inside the task.
3. What you are unsure about.
4. Problems you noticed outside the task.
