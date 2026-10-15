# Learnly

A self-hosted, headless LMS core for one school or university. Design: [ARCHITECTURE.md](ARCHITECTURE.md).

Implemented so far: the foundation (config, migrations, health, OpenAPI) and identity (users, password login for web and app clients, sessions, admin user management). See `ARCHITECTURE.md` for what is still design.

## Prerequisites

Rust (stable, via rustup) and Docker with the Compose plugin.

## Run

```
cp .env.example .env
cp learnly.example.toml learnly.toml   # set server.public_origin and institution.name
docker compose up -d --wait            # PostgreSQL 18 on :5432
cargo run -p server -- migrate         # apply migrations
cargo run -p server -- create-admin you@example.edu --name "Your Name"
cargo run -p server -- serve           # http://127.0.0.1:8080
```

`create-admin` prints a temporary password once. Log in with it, then change it before anything else
works: the account is locked to `GET /me`, logout and `POST /me/password` until you do.

```
curl -s localhost:8080/api/v1/auth/login -H 'content-type: application/json' \
  -d '{"email":"you@example.edu","password":"<temporary password>","client":"app"}'
# use the returned token as: Authorization: Bearer <token>
```

Other users: `learnly-server create-user <email> --name <name> --role admin|teacher|student`, or
`POST /api/v1/admin/users` as an admin. API docs: `/api/v1/docs`.
Health check: `curl localhost:8080/api/v1/health` or `cargo run -p cli -- health --url http://localhost:8080`.

Every config key can be overridden with `LEARNLY__<SECTION>__<KEY>`, e.g. `LEARNLY__DATABASE__URL`.
For plain-HTTP local development set `auth.cookie_secure = false`; keep it `true` in production.
Behind a reverse proxy set `server.client_ip_header` (e.g. `x-real-ip`) so login rate limits see the client address.

## Test

```
cargo test --workspace                 # needs the running PostgreSQL and DATABASE_URL (see .env)
```

A new file in `migrations/` is embedded at compile time; if a test does not see it, touch
`crates/server/src/app.rs` to force a rebuild.
