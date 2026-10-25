# 0065. Layered crates

Status: Accepted

Supersedes in part: ADR-0001 (the list of modules and the Cargo workspace), ADR-0015 (the gradebook is a module of the `learning` crate; the rules of that ADR stand), ADR-0016 (the module layout; the transaction rules stand), ADR-0036 (the harness is the crate `test-support`).

## Context

Nine modules lived in one crate, `server`. The compiler could not enforce ADR-0001 there: the modules called each other in both directions (`identity` ↔ `teaching`, `org` ↔ `teaching`, and `teaching` ↔ `content` ↔ `assessment` ↔ `gradebook`), their error types wrapped each other, their models embedded each other's types, and a dozen queries read another module's tables directly. The rule "other modules use only what `mod.rs` exposes" held inside one crate by convention, and a test read the source to check what it could.

The project is in development, so Rust APIs, module names and the file layout could change freely, without shims.

## Options

1. **Keep one crate and enforce the layers with a test.** Cheapest. The cycles stay invisible to the compiler, and the test only sees what it greps for.
2. **One crate per module.** The compiler enforces everything. But `teaching`, `content`, `assessment` and `gradebook` call each other both ways (deleting a course purges its submissions and grades, a submission saves a grade, the gradebook reads content). Splitting them needs an indirection (a registry of guards, or events) for every such call, which is much code for no gain.
3. **One crate per layer, the four learning modules in one crate.** Chosen.

## Decision

Dependencies point strictly downward, and Cargo enforces it:

```
server, vumlo-cli
  -> learning   (teaching, content, assessment, gradebook, and the flows above them)
  -> org
  -> auth       (was identity)
  -> events
  -> access     (was policy)
  -> common
```

`db` (migrations and their state, the installation record, the CLI's audit row) is a crate of its own beside the chain; `org` (the installation record), `server`, `vumlo-cli` and `test-support` use it. `test-support` is a dev-dependency only. `api-types`, `config` and `module-api` are unchanged.

**What each crate holds**

- `common`: `Role`, the `text_enum!` macro, `ApiError` with `ValidJson`/`ValidQuery` (Problem Details), cursor pagination.
- `access`: `Actor`, `PendingActor`, the actions and `authorize`. `Actor::new` and `Actor::role` are private to the crate; the extractors are generic over `ActorSource`, which the application state implements.
- `events`: the audit writes, the outbox and its worker.
- `auth`: users, sessions, sign-in (password, Google), the account value types and the password hash, the mailer, `COOKIE_NAME`.
- `org`: terms, groups, subgroups, the installation record.
- `learning`: the four modules, each exposing only its `mod.rs` and writing only its own schema (they may call and read one another), `points`, and the flows that need several modules: `people`, `groups`, `reports`, `audit`.
- `server`: `app`, `main`, `modules`, `health`, `conditional`, `idempotency`, and the application state.

**Calls and reads that went upward became flows in `learning`.** Each is one service function with one transaction, as ADR-0016 requires.

- A person with their groups and courses (`people::user_facts`), the change of a user (`people::update_user`), the overview of a group (`groups::group_overview`), the term statistics (`reports::term_stats`), and the audit log with the names it points at (`audit::list_audit`).
- Changing a user has consequences in the user's courses that only `learning` can see (an owner cannot become a student, a demoted teacher leaves their courses, ADR-0037). `auth` therefore exposes only the primitive `apply_user_update`, which returns an `AppliedUserUpdate` that must be audited in the same transaction; only `learning` calls it, and a test checks it.

**Reads across layers**

- A layer reads the tables of its own layer, and those of a lower layer only through a view the lower layer declared: `auth.people`, `org.term_list`, `org.group_list`, `org.subgroup_list`, `events.audit_entries` (migration 0033). It never names the schema of a higher layer. `teaching.effective_members` stays the one way to read a roster (ADR-0033).
- A layer writes only its own schema.

**Renames.** `identity` → `auth`, including the Postgres schema (migration 0032, `ALTER SCHEMA ... RENAME`); `policy` → `access`; the old idea of a `kernel` is `common`.

**State.** One state per layer (`AuthState`, `OrgState`, `LearningState`), kept by `AppState` in `server`, which implements `FromRef` for each. The routers are generic over the state.

**Errors.** Each crate converts its own error enum to `ApiError` inside that crate. No crate wraps the error of a crate above it, and no model embeds a type of a crate above it.

**Tests.**

- A unit test lives next to the code it tests, in the crate that owns the code. The integration tests stay in `server`, because they start the whole application; the CLI's tests stay in `vumlo-cli`.
- The database harness is the crate `test-support`. A crate shares test helpers through a Cargo feature `testing` (`Actor::for_tests`, `auth::create_user_unaudited_for_tests`, `org::seed_group`), which another crate may enable only under `[dev-dependencies]`.
- The release archive leaves out `test-support` and every `tests.rs`, and `verify-archive.sh` checks it.
- `backend/crates/server/tests/architecture.rs` checks what Cargo cannot see: a unit writes only its own schema, reads only its own layer and the declared views below it, the tables of the roster are read only by their owner, `Actor::role` is read only in `access`, `apply_user_update` is called only from `learning`, and `testing` is enabled only for tests.

**The HTTP contract does not change.** The OpenAPI document was identical before and after every step.

## Consequences

- A crate cannot import one above it: the cycles are gone, and `cargo tree -p access` shows that the permission rules do not depend on the database.
- A lower layer is built and tested without the layers above it.
- **Inside `learning` the four modules still call one another in both directions.** The compiler does not see that; only the architecture test limits what they write. This is a deliberate trade (option 2 above), not a clean layering.
- `events.audit_log.actor_id` has a foreign key to `auth.users`, although `events` is below `auth`. Migrations are append-only, so the table stays as it is; it is harmless while the audit report reads the log only through `events.audit_entries`.
- The views are plain pass-throughs. Making one more column of a lower layer readable above it takes a migration.
- The term statistics are two reads (the terms from `org`, the counts from `teaching`) instead of one query, so they no longer come from one snapshot. That is acceptable for a report.
- `vumlo-cli` depends on `auth` for the account value types, so it compiles the web stack of `auth` too.
- A new crate that tests against a database needs `test-support`, and `make-archive.sh` removes that dev-dependency from every manifest.
- The administrator CLI names `auth` and `access`; `vumlo-cli modules list` prints those names where it printed `identity` and `policy`.
