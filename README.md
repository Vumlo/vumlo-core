# Learnly

A self-hosted, headless LMS core for one school or university. Design: [ARCHITECTURE.md](ARCHITECTURE.md).

## Prerequisites

Rust (stable, via rustup) and Docker with the Compose plugin.

## Run

```
cp .env.example .env
cp learnly.example.toml learnly.toml
docker compose up -d --wait            # PostgreSQL 18 on :5432
cargo run -p server -- migrate         # apply migrations
cargo run -p server -- serve           # http://127.0.0.1:8080
```

Then `curl localhost:8080/api/v1/health` (API docs: `/api/v1/docs`) or
`cargo run -p cli -- health --url http://localhost:8080`.

## Test

```
cargo test --workspace                 # needs the running PostgreSQL
```
