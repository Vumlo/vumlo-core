# 0016. Module layout and transaction handling

Status: Accepted. Superseded in part by [ADR-0065](0065-layered-crates.md): the module layout (now spread over crates); the transaction rules stand.

## Context
ADR-0001 defines modules with owned schemas, and ADR-0007 requires a state change and its event to commit together, often across modules. We need a code layout that keeps module boundaries enforceable, and a way to pass one transaction through several modules.

## Options — layout
1. **Module = folder with layer files** (`mod`, `model`, `routes`, `service`, `repo`).
2. Global layers (`routes/`, `services/`, `repos/`) — modules smeared across folders (the LearnSystem layout).
3. One file per use case (vertical slices) — SQL and logic spread everywhere, duplication.
4. One crate per module — strongest boundaries, too much overhead now.

## Options — database access
1. **Pass `&mut PgConnection` explicitly**; the handler owns the transaction.
2. Services hold the pool and open their own transactions — breaks cross-module atomicity and the outbox.
3. Repository traits / Unit of Work — mockable, but a lot of code we don't need: Postgres is fixed and tests run on a real DB.
4. Implicit ambient transaction (`@Transactional`-style) — hidden, not idiomatic.
5. An extractor that opens and auto-commits a transaction per request — hides the commit point and opens transactions for reads.

## Decision

Layout:
- `backend/crates/server/src/{lib.rs, main.rs, app.rs, error.rs, actor.rs, settings.rs}` + one folder per module.
- `server` is a library plus a thin binary `learnly-server`. `lib.rs` only declares modules, `main.rs` is the CLI entry, and integration tests use the library.
- Each module: `mod.rs` (the only public surface, incl. `pub fn router()`), `model.rs` (public types), and private `routes.rs` (a folder `routes/` once it grows), `service.rs`, `repo.rs`.
- `app.rs` only nests module routers. Large files become folders without changing the public surface.
- Later, a module can become its own crate one-to-one.

Transactions:
- Service and repo functions take `&mut PgConnection` and never `begin` or `commit`.
- Writes: the handler or CLI command opens a transaction, calls exactly one *writing* service function, and commits. Reads and slow preparation (loading a user, verifying and hashing passwords) are separate service calls made before `begin`. Cross-module work is orchestrated by the owning module's service, which passes the same connection on.
- Reads: a pooled connection without a transaction. A read-only transaction only for multi-query consistent snapshots.
- No slow work inside a transaction: password hashing before `begin`, no HTTP calls, files stored before metadata is inserted (orphans cleaned by a job).

## Consequences
- The compiler enforces module boundaries even in one crate, because private files are not reachable from other modules.
- Transaction scope is visible in every signature. A forgotten `commit` rolls back (safe failure, caught by tests).
- Handlers stay thin, and business orchestration lives in services.
