# 0036. Each test gets its own database and its own pool

Status: Accepted
Amends: ADR-0014 (Testing)

## Context
ADR-0014 uses `#[sqlx::test]`: a fresh database per test. The audit of task 014 (A-10) found that the
suite stalls when more than about a dozen tests run at once (`cargo test` runs as many as the
machine has cores). A request waits 30 seconds for a database connection, the server answers
`408`, and every database session is idle meanwhile.

The cause is inside `sqlx::test`. Every test pool is a child of one master pool of 20 permits and
steals its permits from it. A child keeps what it stole until it is dropped, the master pool keeps its
own idle connections for ten minutes, and a test that needs one more connection than it has waits for
a permit that only another waiting test could release. The suite passed on 8 cores and failed on
machines with more, in a way that looked like a server bug.

## Options
- Cap the number of test threads: hides the cause, and the next machine with a different core count
  or a slower database meets it again.
- Keep `#[sqlx::test]` and make every test use one connection: tests that hold a transaction open
  while a second writer waits (the lock-order tests of ADR-0022, ADR-0030, ADR-0035) need two or three.
- **A small harness of our own** (`db_test`): the test creates its database over one short
  connection to the maintenance database, runs the migrations, hands the body a pool that belongs
  to that test alone, closes the pool and drops the database with `WITH (FORCE)`.

## Decision
1. Database tests are `#[tokio::test]` functions that run their body in `db_test(async |pool| { … })`
   (`bare_db_test` for the few that must see an empty database). `#[sqlx::test]` is not used.
2. The harness is one file, `backend/crates/server/src/test_support.rs`. The library compiles it for its
   unit tests (`#[cfg(test)]`), and `tests/common` includes the same file by path for the
   integration tests. It adds no crate.
3. `DATABASE_URL` supplies the server and the credentials only. The harness connects to the
   `postgres` maintenance database, so the test run no longer needs the development database to
   exist. (`query!` still needs a database or `SQLX_OFFLINE=true` at compile time.)
4. A database is named `t_<unix seconds>_<uuid>`. A test that fails keeps its database for
   inspection; the first test of the next run drops those older than an hour.
5. A test pool holds at most five connections. PostgreSQL must allow the sum over the tests that
   run at once, so `compose.yaml` raises `max_connections` to 300.
6. A test never holds a pooled connection across a request to the server: it takes one, uses it and
   gives it back in the same statement. A test that waits for a lock uses
   `test_support::wait_until_blocked`, which polls `pg_stat_activity`, never a fixed sleep.
7. The definition of done runs the suite also with `--test-threads=16`.

## Consequences
- The suite passes at 8, 16 and 32 threads.
- Each test pays one `CREATE DATABASE` and the migrations, as before.
- A panicking test leaves a database behind until the next run sweeps it.
- AGENTS §12 and §14 name the harness.
