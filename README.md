# Vumlo

A self-hosted LMS for one school or university: courses, groups and rosters, lessons, assignments
(text, quizzes, forms), a gradebook with published grades, and sign-in by password or Google. It is a
Rust API and worker on PostgreSQL, a Next.js web app, and `vumlo-cli`, the tool that installs and
checks it. What changed in each release: [CHANGELOG.md](CHANGELOG.md).

## Requirements

- **Docker path** (recommended): a Linux server with Docker and the Compose plugin 2.24 or newer, and
  several GB of RAM (or swap) for the one-time Rust build. Nothing else is installed on the host.
- **Linux server path:** PostgreSQL 18, a Rust toolchain, Node 24, Caddy for TLS and systemd.
- For a real installation: a domain name, and an SMTP relay for sign-up and password-reset mail.

## Quick start

Download `vumlo-<version>-source.tar.gz` and `SHA256SUMS` from the
[releases page](https://github.com/Vumlo/vumlo-core/releases), then:

```sh
sha256sum --check --ignore-missing SHA256SUMS && tar xzf vumlo-*-source.tar.gz && cd vumlo-*/
./vumlo init && cp deploy/env.example .env && chmod 600 .env   # then edit vumlo.toml and .env
./vumlo config check && ./vumlo brand apply                    # your name and logo (brand/logo.svg) go into the web app
export COMPOSE_FILE=deploy/compose.yaml COMPOSE_ENV_FILES=.env && docker compose build
# then migrate, create the administrator and start: docs/install.md, "Then, in this order"
```

The full guide, with what to change for a real installation (domain, TLS, mail), upgrades and backups:
[docs/install.md](docs/install.md).
