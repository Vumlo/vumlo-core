# Installing Vumlo

Vumlo is one core (the API and a worker), one web app and one PostgreSQL database, for one school or
university. `vumlo-cli` is the administrator tool that prepares them: the configuration, the generated
branding, the database and the first administrator. It is not a deployment tool: the systemd units, the
Caddyfile and the firewall rules are templates in `deploy/`, and you install them.

There are two ways to run it. Pick one:

- **[Docker only](#path-1-docker-only)**: PostgreSQL, the core, the worker and the web app in
  containers; the only things on the host are Docker and a clone of this repository.
- **[A Linux server](#path-2-a-linux-server)**: PostgreSQL from your distribution, binaries you build
  (or copy), systemd for the services and Caddy for TLS.

Everything that follows is for one institution per installation. The institution's mode, `school` or
`university`, is chosen before the first start and cannot be changed afterwards.

> **Theming.** Vumlo has no theme settings: a university that wants a different look edits `web/`
> itself, which may conflict with upstream updates; a change of `vumlo.toml` means `brand apply`, a web
> rebuild and a restart.

## How the configuration works

`vumlo.toml` holds the settings. The secrets are never in it; they are environment variables, usually in
a file named `.env` that only its owner can read:

| Secret | Variable |
|---|---|
| PostgreSQL connection | `DATABASE_URL` |
| Google sign-in client secret | `VUMLO__AUTH__GOOGLE__CLIENT_SECRET` |
| SMTP password | `VUMLO__MAIL__TRANSPORT__PASSWORD` |

Precedence, lowest first: the defaults, `vumlo.toml`, `VUMLO__SECTION__KEY` environment variables,
`--set section.key=value` flags. `vumlo-cli config check` lists every problem at once: an unknown key, a
secret written in the file, a time zone that does not exist, an origin that browsers could never send.
`vumlo-cli config show` prints the effective configuration with the secrets redacted; `--public` prints
only what reaches the browser.

What reaches the browser is decided by the schema: every key is marked `public`, `private` or `secret`,
and `brand apply` writes only the public ones into the web build.

## Path 1: Docker only

You need Docker with the Compose plugin (version 2.24 or newer). Nothing else is installed on the host.

```sh
git clone https://github.com/Vumlo/vumlo-core.git vumlo && cd vumlo

./vumlo init            # writes vumlo.toml and .env.example; builds the vumlo-cli image the first time
cp .env.example .env && chmod 600 .env
```

`./vumlo` is a small script that runs `vumlo-cli` inside the compose service when there is no local
binary. Edit the two files:

- `.env`: set a real `POSTGRES_PASSWORD`, and put the same password in `DATABASE_URL` (its host is
  `postgres`, the name of the compose service).
- `vumlo.toml`: set at least `institution.name`, `institution.mode` (`"school"` or `"university"`;
  this cannot be changed later), `server.public_origin` (the address people will type; for a trial
  `http://localhost:3000`) and `auth.allowed_domains` (who may sign up, e.g. `["example.edu"]`). For a
  trial over plain HTTP also set `[auth] cookie_secure = false`; keep it `true` for anything real.
  Put the logo in the repository and set `institution.logo = "brand/logo.svg"` (PNG, JPEG, WebP or
  SVG; an SVG must have its text converted to paths).

Then, in this order:

```sh
./vumlo config check                                  # every problem, or "configuration OK"
./vumlo brand apply                                   # writes .vumlo/generated/: the web build reads only this

export COMPOSE_FILE=deploy/compose.yaml COMPOSE_ENV_FILES=.env
docker compose up -d --wait postgres
./vumlo db migrate                                    # applies the migrations, records the mode
./vumlo admin create you@example.edu --name Your --surname Name   # asks for the password twice
docker compose up -d --build                          # core, worker, web (builds the web app from .vumlo/generated)
./vumlo doctor
```

Open `public_origin` and sign in with the administrator. The web app is on `127.0.0.1:3000` and the API
on `127.0.0.1:8080`; for a real domain with TLS put Caddy in front of both (`deploy/Caddyfile`, see
[Caddy](#caddy)). `doctor` reports the web build as "not built" inside the container because the web
app is built into its own image; that is expected for this path, and it warns about `node` for the same
reason.

To change the configuration later: edit `vumlo.toml`, `./vumlo config check`, `./vumlo brand apply`,
then `docker compose up -d --build` (the web image is rebuilt from the new `.vumlo/generated`).

### Demo data

`./vumlo seed --demo` fills an empty installation with a course, a teacher, three students, a quiz and
grades so that there is something to look at. It refuses a database that already holds courses or
people unless you pass `--yes`, and it does nothing a second time.

## Path 2: A Linux server

This is written for a current Debian or Ubuntu with systemd. The same steps work elsewhere with
different package names.

### 1. PostgreSQL

Install PostgreSQL from your distribution (18 is what the project is tested with) and create the role
and the database:

```sh
sudo -u postgres psql -c "CREATE ROLE vumlo LOGIN PASSWORD 'choose-a-password'"
sudo -u postgres psql -c "CREATE DATABASE vumlo OWNER vumlo"
```

### 2. The binaries

You need the two binaries, `vumlo-server` and `vumlo-cli`, built from the same checkout.

**Build them on the server.** Install Rust (`rustup`) and Node 24, then:

```sh
git clone https://github.com/Vumlo/vumlo-core.git && cd vumlo-core/backend
SQLX_OFFLINE=true cargo build --release -p server -p vumlo-cli --features "$VUMLO_FEATURES"
```

`VUMLO_FEATURES` lists the optional modules to compile in, comma separated; leave it empty for none (see
[Optional modules](#optional-modules)). The query data in `backend/.sqlx` is checked in, so no database
is needed to build.

**A small host.** Compiling Rust in release mode needs a good deal more memory than running Vumlo. If
the server has little RAM, either add swap for the build (`fallocate -l 4G /swapfile && mkswap /swapfile
&& swapon /swapfile`) and build with `CARGO_BUILD_JOBS=1`, or build on another machine of the same
operating-system release and copy the two files in `backend/target/release/`. Nothing else is needed to
run them.

Install them:

```sh
sudo useradd --system --home-dir /opt/vumlo --shell /usr/sbin/nologin vumlo
sudo install -d /opt/vumlo/bin
sudo install -m 0755 target/release/vumlo-server target/release/vumlo-cli /opt/vumlo/bin/
sudo ln -sf /opt/vumlo/bin/vumlo-cli /usr/local/bin/vumlo-cli
```

### 3. Configuration

```sh
sudo install -d -m 0750 -o root -g vumlo /etc/vumlo
cd /etc/vumlo
sudo vumlo-cli init                                   # vumlo.toml and .env.example
sudo mv .env.example vumlo.env && sudo chown root:vumlo vumlo.env && sudo chmod 640 vumlo.env
sudoedit /etc/vumlo/vumlo.env                         # DATABASE_URL with the host localhost, and the secrets you use
sudoedit /etc/vumlo/vumlo.toml
```

In `vumlo.toml` set `institution.name`, `institution.mode`, `server.public_origin =
"https://lms.example.edu"`, `server.client_ip_header = "x-real-ip"` (Caddy sets it), `auth.allowed_domains`,
and `[cli] env_file = "vumlo.env"` so the CLI finds the secrets. Set `web.dir = "/opt/vumlo/web"`.
Then:

```sh
sudo vumlo-cli --config /etc/vumlo/vumlo.toml config check
```

### 4. The database and the first administrator

```sh
sudo vumlo-cli --config /etc/vumlo/vumlo.toml db migrate
sudo vumlo-cli --config /etc/vumlo/vumlo.toml admin create you@example.edu --name Your --surname Name
```

### 5. The web app

Copy the `web/` directory of the checkout to `/opt/vumlo/web`, and let the CLI generate the branding and
run the build there:

```sh
sudo rsync -a --delete ../web/ /opt/vumlo/web/
sudo vumlo-cli --config /etc/vumlo/vumlo.toml build web      # brand apply, then npm ci (once) and npm run build in web.dir
sudo chown -R vumlo: /opt/vumlo/web/.next/cache
```

### 6. systemd

```sh
sudo cp ../deploy/systemd/vumlo-*.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now vumlo-core vumlo-worker vumlo-web
sudo vumlo-cli --config /etc/vumlo/vumlo.toml doctor
```

The units run as the `vumlo` user with the hardening options of `systemd-analyze security` (no
capabilities, a read-only file system except what each needs, no new privileges, a system-call filter).
The web unit leaves out `MemoryDenyWriteExecute` because Node's JIT needs it. Edit the paths at the top of
each unit if you installed elsewhere.

### Caddy

Caddy gives the web app and the API one origin and gets the TLS certificate:

```sh
sudo apt install caddy
sudo cp ../deploy/Caddyfile /etc/caddy/Caddyfile
echo 'VUMLO_DOMAIN=lms.example.edu' | sudo tee -a /etc/default/caddy
sudo systemctl restart caddy
```

Open ports 80 and 443 only. Vumlo itself listens on `127.0.0.1:8080` (the API) and `127.0.0.1:3000` (the
web app); nothing else should reach them. The firewall is yours to configure; `vumlo-cli` does not touch it.
The Caddyfile of this repository also serves the Docker install: its two upstreams are the same ports.

### Backups, and a restore test

A backup you have never restored is a hope. Take one and prove it reads back:

```sh
set -a; . /etc/vumlo/vumlo.env; set +a
PGDATABASE="$DATABASE_URL" pg_dump --format=custom --file vumlo-$(date -u +%F).dump
pg_restore --list vumlo-*.dump > /dev/null && echo "the dump is readable"
```

Restore it into a scratch database and look at it with the CLI, without touching production:

```sh
sudo -u postgres createdb vumlo_restore_test
sudo -u postgres pg_restore --no-owner --role=vumlo -d vumlo_restore_test vumlo-*.dump
DATABASE_URL=postgres://vumlo:choose-a-password@localhost/vumlo_restore_test \
  vumlo-cli --config /etc/vumlo/vumlo.toml db status
sudo -u postgres dropdb vumlo_restore_test
```

`db status` should show every migration applied and none pending. Run the dump nightly from cron or a
systemd timer and keep the last several; copy them off the machine. To restore for real: stop the three
services, drop and recreate the database, `pg_restore` into it, start the services, run `doctor`.

### Upgrades

Get the new release, build it and install the two binaries and the web app as above, then run:

```sh
deploy/update.sh --systemd
```

It checks the configuration, takes a backup and reads it back, stops the services, runs `db migrate`,
starts the services and runs `doctor`. It stops at the first failure, and it never migrates before the
backup exists. If it stops after the services were stopped, they stay stopped: fix the cause and run it
again, or restore the backup. For Docker the same script is `deploy/update.sh --docker`.

`doctor` fails when the CLI, the server and the web app are not the same release, so a half-finished
upgrade does not go unnoticed.

## Optional modules

Some features are modules that not every institution wants (none are included yet in a default build).
A module is compiled in by a Cargo feature of the same name and switched on in `vumlo.toml`:

```toml
[modules]
some-module = true
```

Both halves are needed. **Adding or removing a module means rebuilding with its feature**, for the server
and for `vumlo-cli` in one set: `VUMLO_FEATURES=some-module` in the Docker build (`.env`), or `--features
some-module` in the cargo command above. Editing `[modules]` alone is not enough, and `config check`
says so ("build with the Cargo feature ..."). `vumlo-cli modules list` shows each module as `enabled`,
`disabled` (compiled in, off) or `unavailable` (not compiled in). A module that is off mounts nothing and
loses nothing: its data stays in the database, and `doctor` tells you when data of a module is kept.

The web UI of a module is built into the web app only while the module is on; after changing `[modules]`
run `brand apply` and rebuild the web app.

## Commands

| Command | What it does |
|---|---|
| `init [--dry-run]` | writes `vumlo.toml` and `.env.example`; never overwrites |
| `config check` | validates the configuration, lists every problem |
| `config show [--public]` | the effective configuration, secrets redacted |
| `modules list [--json]` | the core modules and the optional ones with their state |
| `module <name> ...` | the subcommands a module brings, while it is on |
| `brand apply [--dry-run]` | regenerates `.vumlo/generated/` from the configuration and the logo |
| `build web [--dry-run]` | `brand apply`, then the web build |
| `db status` | what is applied and pending; non-zero if the schema is newer than the binary |
| `db migrate [--dry-run]` | applies pending migrations; refuses a schema newer than the binary |
| `admin create <email> --name --surname` | creates an administrator; the password is asked for or read from `--password-stdin`, never an argument |
| `seed --demo [--yes] [--dry-run]` | demo data for an empty installation |
| `doctor [--json]` | checks everything, changes nothing, non-zero if a check fails |
| `version` | the release, the config version and the modules compiled in |

Every command that changes the database writes `events.audit_log` (no actor, source `cli`): `db migrate`,
`admin create` and `seed --demo`. The audit log is readable by administrators in the web app.

## When `doctor` fails

| Line | Usually |
|---|---|
| `configuration` | `vumlo-cli config check` has the details |
| `environment` | `DATABASE_URL` is not in the environment or the env file |
| `env file` | the secrets file is readable by others: `chmod 600` it |
| `database` | PostgreSQL is not running, or the URL, user or password is wrong |
| `schema` | migrations pending (`db migrate`), or the database is newer than this binary (update the binary) |
| `core` | the API is not running, or `cli.core_url` is wrong for where the CLI runs |
| `versions` | the CLI, the server and the web app are different releases: update them together |
| `module sets` | the CLI and the server were built with different modules: rebuild both with one `VUMLO_FEATURES` |
