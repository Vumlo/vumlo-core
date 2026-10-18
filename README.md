# Learnly

A self-hosted, headless LMS core for one school or university. Design: [ARCHITECTURE.md](ARCHITECTURE.md).

Implemented so far: the foundation (config, migrations, health, OpenAPI), identity (users, password login for web and app clients, sessions, admin user management), courses with terms, groups and the live roster, the content tree with `text` and `no_submission` assignments, the gradebook, and text submissions (drafts, attempts, deadlines and late work, attempt limits). See `ARCHITECTURE.md` for what is still design.

## Prerequisites

Rust (stable, via rustup) and Docker with the Compose plugin.

## Run

The database and the API run in Docker; the web app runs on the host (Node 22+, pnpm).

```
docker compose up -d --build --wait    # PostgreSQL 18 on :5432, API on 127.0.0.1:8080 (migrates first)
docker compose exec server learnly-server create-admin you@example.edu --name Your --surname Name
cd web && pnpm install && pnpm dev     # http://localhost:3000, /api/v1 is proxied to :8080
```

`pnpm dev:mock` runs the web app against in-browser mocks instead, with no backend. Set the institution
before the first start with `LEARNLY_INSTITUTION_MODE` (`school` or `university`, default `university`) and
`LEARNLY_INSTITUTION_NAME` in `.env`; the mode cannot be changed later. If the web app is not on
`http://localhost:3000`, set `LEARNLY_PUBLIC_ORIGIN` to its origin.

Without Docker for the API:

```
cp .env.example .env
cp learnly.example.toml learnly.toml   # set server.public_origin, institution.name and institution.mode
docker compose up -d --wait postgres   # PostgreSQL 18 on :5432
cargo run -p server -- migrate         # apply migrations
cargo run -p server -- create-admin you@example.edu --name Your --surname Name
cargo run -p server -- serve           # http://127.0.0.1:8080
```

`create-admin` prints a temporary password once. Log in with it, then change it before anything else
works: the account is locked to `GET /me`, logout and `POST /me/password` until you do.

```
curl -s localhost:8080/api/v1/auth/login -H 'content-type: application/json' \
  -d '{"email":"you@example.edu","password":"<temporary password>","client":"app"}'
# use the returned token as: Authorization: Bearer <token>
```

Other users: `learnly-server create-user <email> --name <name> --surname <surname> --role admin|teacher|student`, or
`POST /api/v1/admin/users` as an admin. API docs: `/api/v1/docs`.
Health check: `curl localhost:8080/api/v1/health` or `cargo run -p cli -- health --url http://localhost:8080`.

Choose `institution.mode` (`school` or `university`) before the first start: it is required, the first command that uses the database records it, and it cannot be changed later (a different value stops the server). The grading scale follows it unless `institution.grading_scale` is set.

Every config key can be overridden with `LEARNLY__<SECTION>__<KEY>`, e.g. `LEARNLY__DATABASE__URL`.
For plain-HTTP local development set `auth.cookie_secure = false`; keep it `true` in production.
Behind a reverse proxy set `server.client_ip_header` (e.g. `x-real-ip`) so login rate limits see the client address. The login limits (`auth.login_ip_burst` and the others, see `learnly.example.toml`) can be raised for a school behind one NAT. A frontend hosted on another origin is allowed with `server.allowed_origins` (ADR-0041; read the SameSite note in `learnly.example.toml` first).

## Test

```
cargo test --workspace                 # needs the running PostgreSQL and DATABASE_URL (see .env)
cargo test --workspace -- --test-threads=16
```

Each test creates its own database (`t_…`) through `crates/server/src/test_support.rs` and drops it
afterwards (ADR-0036). `DATABASE_URL` only supplies the server and the credentials: the tests connect
to the `postgres` database, so the development database does not have to exist. A test that fails
keeps its database; the next run drops the ones older than an hour. Building without a database:
`SQLX_OFFLINE=true`.
