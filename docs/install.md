# Installing Vumlo

Vumlo is one core (the API and a worker), one web app and one PostgreSQL database, for one school or
university. `vumlo-cli` is the administrator tool that prepares them: the configuration, the generated
branding, the database and the first administrator. It is not a deployment tool: the systemd units, the
Caddyfile and the firewall rules are templates in `deploy/`, and you install them.

There are two ways to run it. Pick one:

- **[Docker only](#path-1-docker-only)**: PostgreSQL, the core, the worker and the web app in
  containers; the only things on the host are Docker and the unpacked release archive.
- **[A Linux server](#path-2-a-linux-server)**: PostgreSQL from your distribution, binaries you build
  (or copy), systemd for the services and Caddy for TLS.

`npx create-vumlo` does the first two steps of the Docker path for you: it downloads the newest release, checks it
against its `SHA256SUMS`, keeps only the parts you pick (web app, desktop app, optional modules, plugins), asks for the
institution's name, short name, colour, mode, address and who may sign up, and writes `vumlo.toml` and `.env` (with a
generated database password). It then prints the commands of step 3. It needs Node 20 or newer. Nothing in it is
required: the steps below do the same by hand. Later, `npx create-vumlo update`, run inside the installation, updates it
in place (see [Upgrading](#upgrading)).

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
| S3 access key and secret key (only with `files.storage = "s3"`; ADR-0069) | `VUMLO__FILES__S3__ACCESS_KEY`, `VUMLO__FILES__S3__SECRET_KEY` |
| Plugin key (32 characters or more; ADR-0066) | `VUMLO__PLUGINS__SECRET_KEY` |

Precedence, lowest first: the defaults, `vumlo.toml`, `VUMLO__SECTION__KEY` environment variables,
`--set section.key=value` flags. `vumlo-cli config check` lists every problem at once: an unknown key, a
secret written in the file, a time zone that does not exist, an origin that browsers could never send.
`vumlo-cli config show` prints the effective configuration with the secrets redacted; `--public` prints
only what reaches the browser.

What reaches the browser is decided by the schema: every key is marked `public`, `private` or `secret`,
and `brand apply` writes only the public ones into the web build.

## Path 1: Docker only

You need Docker with the Compose plugin (version 2.24 or newer). Nothing else is installed on the host.

### 1. Download and verify the release

Take the newest release from the [releases page](https://github.com/Vumlo/vumlo-core/releases) and read
its entry in `CHANGELOG.md` (the **Upgrade notes**). Download `vumlo-<version>-source.tar.gz` and
`SHA256SUMS` (`0.1.0` below), check the archive and unpack it:

```sh
sha256sum --check --ignore-missing SHA256SUMS         # must say: vumlo-0.1.0-source.tar.gz: OK
tar xzf vumlo-0.1.0-source.tar.gz && cd vumlo-0.1.0
```

Do not continue if the check fails. Everything below is run from this directory.

### 2. Configure

```sh
./vumlo init            # writes vumlo.toml; builds the vumlo-cli image the first time (a few minutes)
cp deploy/env.example .env && chmod 600 .env
```

`./vumlo` is a small script that runs `vumlo-cli` inside the compose service when there is no local
binary. Edit the two files:

- `.env`: set a real `POSTGRES_PASSWORD`, and put the same password in `DATABASE_URL` (its host is
  `postgres`, the name of the compose service).
- `vumlo.toml`: set at least `institution.name`, `institution.mode` (`"school"` or `"university"`;
  this cannot be changed later), `server.public_origin` (the address people will type; for a trial
  `http://localhost:3000`), `server.admin_origin` (the address of the admin console, always another
  address than `public_origin`; for a trial `http://localhost:3001`) and `auth.allowed_domains` (who may sign up, e.g. `["example.edu"]`). The
  file starts with `[auth] cookie_secure = false`, which is only for a trial over plain HTTP: delete
  that line for anything real.

The release ships with the name "Vumlo". Set `institution.name` and `institution.short_name` to your
institution's: the web app and the desktop app are then called "Vumlo | Short name". The logo is
always Vumlo's (`brand/vumlo-mark.png` and `brand/vumlo-logo.png`); it is not configurable. The colour is:
`institution.accent` is `violet` (the default), `blue`, `teal`, `green`, `orange` or `red`.

### 3. Build and start

```sh
./vumlo config check                                  # every problem, or "configuration OK"
./vumlo brand apply                                   # writes web/src/generated and web/public/brand: the web build reads only these

export COMPOSE_FILE=deploy/compose.yaml COMPOSE_ENV_FILES=.env
docker compose build                                  # core, worker and the web app, from this directory
docker compose up -d --wait postgres
./vumlo db migrate                                    # applies the migrations, records the mode
./vumlo admin create you@example.edu --name Your --surname Name   # asks for the password twice
docker compose up -d
./vumlo doctor
```

**A small server.** Compiling the Rust core is a one-time job that needs a good deal more memory than
running Vumlo. If `docker compose build` is killed or the server freezes, either add swap for the build
(`fallocate -l 4G /swapfile && mkswap /swapfile && swapon /swapfile`), or build on a bigger machine of
the same CPU type with the same archive and `docker compose build`, then copy the images over:
`docker save vumlo-backend:0.1.0 vumlo-web:0.1.0 | gzip | ssh server docker load` (the images are named
after the release). Run `./vumlo brand apply` on the
build machine too.

Open `public_origin` and sign in with the administrator. The web app is on `127.0.0.1:3000`, the admin
console (the administrator's own address and sign-in, `admin_origin`) on `127.0.0.1:3001` and the API
on `127.0.0.1:8080`; administration is done in the console, not in the web app; for a real domain with TLS put Caddy in front of both (`deploy/Caddyfile`, see
[Caddy](#caddy)). `doctor` reports the web build as "not built" inside the container because the web
app is built into its own image; that is expected for this path, and it warns about `node` for the same
reason.

To change the configuration later: edit `vumlo.toml`, `./vumlo config check`,
`./vumlo brand apply`, then `docker compose up -d --build` (the web image is rebuilt from the new
`web/src/generated` and `web/public/brand`, which `brand apply` wrote).

### Upgrading

Read the **Upgrade notes** of every release between yours and the new one in `CHANGELOG.md`. Then, inside the
installation:

```sh
npx create-vumlo update         # downloads the newest release, checks it, replaces the code in place
./vumlo config check            # a new config_version or key is listed here
deploy/update.sh --docker       # brand apply, build, back up, stop, migrate, start, doctor
```

`npx create-vumlo update` copies the files of the old release it replaces into
`backups/vumlo-<old version>-files.tar.gz` first, so an edit you made to `web/` is not lost. It never touches
`vumlo.toml`, `.env`, `backups/` or `plugins/`, and it keeps what you had left out (no `web/` stays without one).
Without Node: download `vumlo-<version>-source.tar.gz` and `SHA256SUMS`, check them with
`sha256sum --check --ignore-missing SHA256SUMS`, unpack the archive, and copy its files over the installation, except
the four above.

`update.sh` stops at the first failure and never migrates before the backup exists (see
[Upgrades](#upgrades) for what it does). The database lives in a Docker volume that does not depend on
the directory. If `plugins/compose.yaml` exists (`npx create-vumlo` writes it when you pick plugins), `./vumlo` and
`update.sh` start and rebuild the plugin containers with the rest. A plugin in a container is installed with
`./vumlo plugin install http://plugin-<name>:8000`: a host name without a dot may use `http://`, because it cannot
name a host on the public internet.

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

You need the two binaries, `vumlo-server` and `vumlo-cli`, built from the same archive.

**Build them on the server.** Install Rust (`rustup`) and Node 24, then:

```sh
cd vumlo-0.1.0/backend        # the unpacked, verified archive of step 1 of the Docker path
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
sudo mv .env.example vumlo.env && sudo chmod 600 vumlo.env     # root only: systemd reads it as root
sudoedit /etc/vumlo/vumlo.env                         # DATABASE_URL with the host localhost, and the secrets you use
sudoedit /etc/vumlo/vumlo.toml
```

In `vumlo.toml` set `institution.name`, `institution.mode`, `server.public_origin =
"https://lms.example.edu"`, `server.admin_origin = "https://admin.lms.example.edu"` (the admin
console's own address, see [Caddy](#caddy)), `server.client_ip_header = "x-real-ip"` (Caddy sets it), `auth.allowed_domains`,
and in `[cli]` set `env_file = "vumlo.env"` (so the CLI finds the secrets) and `core_url =
"http://127.0.0.1:8080"` (so `doctor` asks the core directly and not through Caddy). Set `web.dir =
"/opt/vumlo/web"`. Remove the `cookie_secure = false` line, or set it to `true`: the site is HTTPS.
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

Copy the `web/` directory of the archive to `/opt/vumlo/web`, and let the CLI generate the branding and
run the build there:

```sh
sudo rsync -a --delete ../web/ /opt/vumlo/web/
sudo vumlo-cli --config /etc/vumlo/vumlo.toml build web      # brand apply, then npm ci (once) and npm run build in web.dir
sudo chown -R vumlo: /opt/vumlo/web/.next/cache
```

The admin console is built by the same command, from `web-admin/` beside `web/`: copy it too
(`sudo rsync -a --delete ../web-admin/ /opt/vumlo/web-admin/`, before the `build web` above). It is static
files in `/opt/vumlo/web-admin/dist`, which Caddy serves; it has no process of its own.

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

The admin console has an address of its own, `admin.lms.example.edu` (`server.admin_origin`): give it a DNS
record next to the main one and the same variable in Caddy's environment as the main domain:

```sh
echo 'VUMLO_ADMIN_DOMAIN=admin.lms.example.edu' | sudo tee -a /etc/default/caddy
```

Without Docker, the console is its built files: in the second block of the Caddyfile replace the
`reverse_proxy 127.0.0.1:3001` with the `root` / `file_server` lines its comment shows. Administrators sign in
there with their email and password; if one has forgotten the password,
`vumlo-cli admin set-password <email>` sets a new one.

Open ports 80 and 443 only. Vumlo itself listens on `127.0.0.1:8080` (the API), `127.0.0.1:3000` (the
web app) and, with Docker, `127.0.0.1:3001` (the admin console); nothing else should reach them. The firewall
is yours to configure; `vumlo-cli` does not touch it. The Caddyfile of the release also serves the Docker
install: its upstreams are the same ports.

### Backups, and a restore test

A backup you have never restored is a hope. Take one and prove it reads back:

```sh
sudo vumlo-cli --config /etc/vumlo/vumlo.toml db backup --output /var/backups/vumlo-$(date -u +%F).dump
```

`db backup` needs the PostgreSQL client tools (`apt install postgresql-client`). It takes the
connection from `DATABASE_URL`, so the password never appears in the process list, creates the file
readable by its owner only, and reads the dump back with `pg_restore --list` before it says it worked.
It never overwrites a file.

The dump holds the database only. **Uploaded files are not in it**: back up the files directory or the bucket separately and restore the two together (see [Uploaded files](#uploaded-files)).

Restore it into a scratch database and look at it with the CLI, without touching production. The dump
is readable by its owner only, so it is piped in (`postgres` cannot open the file itself):

```sh
sudo -u postgres createdb --owner vumlo vumlo_restore_test
sudo cat /var/backups/vumlo-2026-01-15.dump | sudo -u postgres pg_restore --no-owner --role=vumlo -d vumlo_restore_test
sudo env DATABASE_URL=postgres://vumlo:choose-a-password@localhost/vumlo_restore_test \
  vumlo-cli --config /etc/vumlo/vumlo.toml db status
sudo -u postgres dropdb vumlo_restore_test
```

`db status` should show every migration applied and none pending. Run the dump nightly from cron or a
systemd timer and keep the last several; copy them off the machine. To restore for real: stop the three
services, drop and recreate the database (`dropdb`, then `createdb --owner vumlo`), pipe the dump into
`pg_restore` as above with `-d vumlo`, start the services, run `doctor`.

### Upgrades

Download, verify and unpack the new archive, read its upgrade notes in `CHANGELOG.md`, build it and
install the two binaries and the web app as above, then run (from the new directory):

```sh
sudo deploy/update.sh --systemd      # as root: the secrets file is root's, and so are the services
```

It checks the configuration, takes a backup and reads it back, stops the services, runs `db migrate`,
starts the services and runs `doctor`. It stops at the first failure, and it never migrates before the
backup exists. If it stops after the services were stopped, they stay stopped: fix the cause and run it
again, or restore the backup. For Docker the same script is `deploy/update.sh --docker`.

`doctor` fails when the CLI, the server and the web app are not the same release, so a half-finished
upgrade does not go unnoticed.

## Mail

The optional `[mail]` section of `vumlo.toml` (ADR-0084 to ADR-0087) turns on every mail the server sends:
the links of self-registration and password reset, and the messages plugins with the scope `mail:send` write
to the people of their courses. Without it register and reset are `404`, and a plugin that tries to send gets `503`.

| Key | Default | Meaning |
|---|---|---|
| `from` | none | the sender, `name@example.edu` or `Vumlo <no-reply@example.edu>` |
| `link_base` | `server.public_origin` | where links in mails point |
| `transport.kind` | none | `smtp` (`host`, `port`, `tls`, `username`; the password is `VUMLO__MAIL__TRANSPORT__PASSWORD`) or `file` (`dir`, for a test) |
| `plugin_hourly_calls` | 500 | accepted mail calls one plugin may make in an hour (1 to 100000) |
| `recipient_daily` | 20 | plugin messages one person may get in 24 hours (1 to 1000); account mail does not count |
| `max_audience` | 2000 | the most people one call may address (1 to 10000); a larger audience is refused with `422` |
| `retention_days` | 30 | days an address is kept after a plugin message ends; account mail is erased at once (1 to 3650) |
| `journal_days` | 180 | days the journal of messages is kept (`retention_days` to 3650) |

Every message is a row, sent by the mail worker (in `serve` and in `worker`) a few seconds after it was queued,
and retried with a growing delay; one that fails for good, or is a day old, is a `dead` row, and
`vumlo-cli mail status`, `mail list` and `mail retry` show and requeue them (see [Commands](#commands)). The journal
shows a person by id and never prints an address. Every plugin mail ends with a footer and a link with which the
person turns off that kind of mail from that plugin, and carries the `List-Unsubscribe` headers, so a mail client
offers its own button; they need no sign-in.

## Uploaded files

Teachers attach files to content and students hand in files (ADR-0069). The bytes are not in PostgreSQL;
they are kept by the `[files]` section of `vumlo.toml`:

```toml
[files]
storage = "local"      # "local" (a directory on the server) or "s3"
path = "files"         # local only: relative to the directory of vumlo.toml; default "files"
max_size = 52428800    # bytes, 1 to 1073741824 (1 GiB); the largest file, default 50 MiB

# [files.s3]           # required when storage = "s3"
# endpoint = "https://s3.example.edu"   # an S3-compatible service other than AWS; https only
# bucket = "vumlo-files"
# region = "eu-north-1"
```

- **Local storage.** The server creates the directory when it starts, so the user that runs `serve` and
  `worker` must be able to write there, and both read the same directory. The default, `files` beside
  `vumlo.toml`, would be under `/etc/vumlo` (systemd, read-only under `ProtectSystem=strict`) or in the
  read-only configuration mount (Docker), so the shipped files override it with the environment variable
  `VUMLO__FILES__PATH`, which takes precedence over `files.path` in `vumlo.toml`:
  - **systemd.** `vumlo-core.service` and `vumlo-worker.service` set `StateDirectory=vumlo` (systemd creates
    `/var/lib/vumlo`, owned by `vumlo` and writable despite `ProtectSystem=strict`) and
    `VUMLO__FILES__PATH=/var/lib/vumlo/files`. Back up `/var/lib/vumlo/files`. To keep the files elsewhere,
    point `VUMLO__FILES__PATH` (or `files.path`) at the new directory and make that directory writable for
    both units, for example with `ReadWritePaths=<dir>` in a drop-in, because `ProtectSystem=strict` leaves
    everything else read-only.
  - **Docker.** The `core` and `worker` services mount the named volume `files-data` at
    `/var/lib/vumlo/files` and set `VUMLO__FILES__PATH` to it; the image creates the directory for uid
    10001, so the volume starts out writable. The volume survives `docker compose down` (not
    `down --volumes`).
  - **Without the shipped files** (a development run), the default is `files` beside `vumlo.toml`; it is
    ignored by git. `serve` and `worker` refuse to start if the directory cannot be created.
- **S3.** `storage = "s3"` with the `[files.s3]` section. The keys are secrets and only come from the
  environment: `VUMLO__FILES__S3__ACCESS_KEY` and `VUMLO__FILES__S3__SECRET_KEY`. `config check` names
  what is missing.
- **Size.** A file is read into memory before it is stored, so the server needs `max_size` of memory for each
  upload in progress; keep the value as low as your files allow. A larger file is refused with `413`.
- **Reverse proxy.** The proxy in front of the API must let a request body of `max_size` through. The Caddyfile
  of this release sets no limit (`request_body` is off by default in Caddy) and sends `/api/*` straight to the
  API, so it needs no change. With nginx, set `client_max_body_size` to at least `max_size` (the default is
  1 MB); other proxies have a similar setting.
  The web app's own `/api/v1` forward is the path of `npm run dev` and also the production path of the Docker
  setup without Caddy; it was not tested with large bodies.
- **Backups.** `db backup` is a `pg_dump` and does **not** contain the files. Back up the directory (or the
  bucket) on its own schedule, and restore it together with the database taken at about the same time: a
  database restored without its files makes the download fail with a logged `500` (the row is there, the
  object is not), and objects without their rows are never reachable and nothing deletes them (the
  sweep walks rows, not the directory or the bucket). `deploy/update.sh` backs up the database
  only.
- **Orphans.** A file that was uploaded but that no item and no submission names (an abandoned upload, a
  file removed from a document) is deleted after 24 hours. The sweep runs every hour in the worker, and in
  `serve` unless `--no-worker`. Nothing else deletes files: a deleted course takes its references, and the
  sweep takes the files once nothing names them. An upload whose row could not be saved (for example
  because the submission window closed while the file was on its way) removes its own object at once.

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

### The Buddy (`ai_agent`)

A chat helper on every page for students, teachers and admins ([ADR-0070](decisions/in-progress.md)). Build
with the feature `ai_agent`, then:

```toml
[modules.ai_agent]
enabled = true
endpoint = "https://api.openai.com/v1"   # any OpenAI-compatible API, or your own: http://localhost:11434/v1
# allow_insecure_http = true             # only for a model on this same machine
model = "gpt-4o-mini"
roles = "teacher,admin"                  # who gets it: student, teacher, admin (default: teacher,admin)
daily_cap = 100                          # messages per person per day; 0 = no limit
retention_days = 30                      # how long a chat is kept
notice = "Your messages and the course data they ask about are sent to Example AI."
```

The key goes in the environment, never in the file: `VUMLO_AI_AGENT_API_KEY` (not needed for a model that
has no key). The Buddy looks things up as the person who asks, so it shows nothing they could not open
themselves. A student's Buddy explains and coaches; it can also prepare saving a draft of a text assignment or submitting the saved draft, but only words the student wrote in the chat (it cannot write or change an answer), and only the student's Confirm does it. A teacher's or admin's Buddy can also *prepare* a change (save a draft grade, publish grades, set or remove a deadline, create a draft section, page, assignment or quiz; what it creates is never published; for an admin also create a term or a group, add students to a group, and give a person a temporary password); it happens only when that person presses Confirm, as them, so the usual rules and audit rows apply. **What leaves your server:** the
person's message, the earlier messages of their chat, and what the lookups return (for a teacher that
includes the names of students whose work waits for grading; for an admin, the names, emails and roles of people they ask about). A temporary password never leaves your server: the person who confirms sees it in the panel, once, and the model never does. Say so in `notice`; it is shown on the first
open. A person's chat, and the changes it prepared, are readable by nobody else and are deleted after `retention_days`; "Clear chat" deletes them at once. The desktop app shows the panel too, but only while it is online: it keeps no copy of the chat on the disk, and a message may take minutes (the app waits up to ten).

## Commands

| Command | What it does |
|---|---|
| `init [--dry-run]` | writes `vumlo.toml` and `.env.example`; never overwrites |
| `config check` | validates the configuration, lists every problem |
| `config show [--public]` | the effective configuration, secrets redacted |
| `modules list [--json]` | the core modules and the optional ones with their state |
| `module <name> ...` | the subcommands a module brings, while it is on |
| `brand apply [--dry-run]` | regenerates `web/src/generated/` and `web/public/brand/` from the configuration (nothing to do without a web app) |
| `config schema` | prints the JSON Schema of `vumlo.toml` |
| `build web [--dry-run]` | `brand apply`, then the web build |
| `db status` | what is applied and pending; non-zero if the schema is newer than the binary |
| `db backup [--output FILE]` | a `pg_dump` of the database, read back with `pg_restore`; never overwrites |
| `db migrate [--dry-run]` | applies pending migrations; refuses a schema newer than the binary |
| `admin create <email> --name --surname` | creates an administrator; the password is asked for or read from `--password-stdin`, never an argument |
| `seed --demo [--yes] [--dry-run]` | demo data for an empty installation |
| `mail status [--json]` | what waits in the mail queue, what was sent and what failed in the last 24 hours, the transport |
| `mail list [--state --plugin --kind --user --hours --limit --json]` | the messages, newest first; no address is printed |
| `mail retry <id> \| --dead [--plugin ID] [--dry-run]` | puts dead messages back in the queue (one whose address was already erased is only listed) |
| `facts rebuild [--course ID]` | makes the facts the access rules read (first submission, published score) again from the submissions and grades; the students' own marks stay; holds the facts table while it runs |
| `doctor [--json]` | checks everything, changes nothing, non-zero if a check fails |
| `version` | the release, the config version and the modules compiled in |

Every command that changes the database writes `events.audit_log` (no actor, source `cli`): `db migrate`,
`admin create`, `seed --demo`, `mail retry` and `facts rebuild`. The audit log is readable by administrators in the web app.

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
