# Learnly

A self-hosted, headless LMS core for one school or university. Design: [ARCHITECTURE.md](ARCHITECTURE.md).

Implemented so far: the foundation (config, migrations, health, OpenAPI), identity (users, password login for web and app clients, sessions, admin user management), courses with terms, groups and the live roster, the content tree with `text` and `no_submission` assignments, the gradebook, and text submissions (drafts, attempts, deadlines and late work, attempt limits). See `ARCHITECTURE.md` for what is still design.

## Prerequisites

Rust (stable, via rustup) and Docker with the Compose plugin.

## Run

```
cp .env.example .env
cp learnly.example.toml learnly.toml   # set server.public_origin, institution.name and institution.mode
docker compose up -d --wait            # PostgreSQL 18 on :5432
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
Behind a reverse proxy set `server.client_ip_header` (e.g. `x-real-ip`) so login rate limits see the client address.

## Test

```
cargo test --workspace                 # needs the running PostgreSQL and DATABASE_URL (see .env)
```
