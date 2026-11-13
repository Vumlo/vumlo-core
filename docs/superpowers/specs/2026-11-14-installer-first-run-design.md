# The installer asks what the first run needs: design

Status: built in `create-vumlo` (branch `feat/first-run-setup`, not pushed). ADR-0096. Roadmap item 7. Repository of the work: `create-vumlo`
(one PR); the release side needs nothing.

## 1. Intent

`npx create-vumlo` is the one way an installation is made. Its reader is a devops person who sets a Vumlo up for an
institution and understands Docker, DNS and a reverse proxy. Today it asks about the institution (name, mode, addresses,
language, accent, domains, parts, modules, plugins) and nothing about what the first run depends on:

- no mail: self-registration and password reset answer 404 and mail plugins get 503 until someone edits `vumlo.toml`;
- no file storage choice: `[files]` stays local, S3 is never offered;
- no proxy settings: for a real domain the person reads the guide, appends to `/etc/default/caddy` by hand, and may forget
  `server.client_ip_header`, after which every login limit sees the proxy as one client;
- a wrong value is found late: a bad SMTP password or bucket shows up after the image has been built;
- a failed run is hard to repeat: it removes the folder only if it created it (an existing empty folder keeps a
  half-unpacked release and the retry stops with "is not empty"), and a failed `--start` leaves a Postgres volume with
  the old generated password, so a second run cannot log in;
- a successful `--start` leaves the services running although the person has not yet put the proxy and DNS in place.

Deliver:

1. Questions for mail and file storage (local or S3), written to `vumlo.toml` and `.env`.
2. A check of the answers before anything is written or started: the SMTP host accepts the login, the bucket answers.
3. For https addresses: `server.client_ip_header = "x-real-ip"` and a `caddy.env` with the two domains.
4. A retry that starts over: an unfinished installation is recognised by a marker and removed, with its containers and
   volumes, then installed again.
5. `--start` stops the services when it is done.

Rule for the questions: what the first run needs is asked here; what can change later belongs to the admin console
(ADR-0095), not to this installer.

Goals: a wrong mail or storage setting fails in seconds, before the download is built; a retry always works; no new
dependency.

Non-goals:

- Google sign-in, mail limits, retention, login limits, the Buddy beyond what exists: not needed for a first run, they stay
  in `vumlo.toml` and the guide.
- Installing or configuring Caddy itself, DNS, the firewall, certificates. The installer writes what the Caddyfile reads.
- Resuming a failed install. A failed install is repeated from the start.
- A mail test message. The check logs in and quits; it sends nothing.
- A check of the local storage directory (the guide covers the permission).

Anti-goal: an installer that deletes something that was not its own. Anything that wipes is gated by the marker.

## 2. Approaches

**Checks.** (A) Node built-ins only: a minimal SMTP dialogue on `node:net`/`node:tls` and a signed `HEAD` of the bucket with
SigV4 from `node:crypto`; about 110 lines, no dependency, runs in seconds. (B) `nodemailer` and `@aws-sdk/client-s3`:
well-tested but a large `npx` install for a tool that has had no dependencies, and supply-chain surface on every run.
(C) `vumlo-cli mail check` and a files check in the core: the same code that sends later does the check, but it needs
the built image, which is what the check must precede. **Chosen: A.** The cost is two small protocol clients to own; the
mitigations are in section 6.

**Retry.** (A) Marker file, removed at the very end; a folder with the marker is wiped, containers and volumes included.
(B) Wipe whatever folder the person names, after a confirmation. (C) Resume from where it failed. **Chosen: A**: B can
delete a finished installation, C is a state machine for a case the owner wants repeated from scratch.

**Proxy.** (A) Write the settings, leave Caddy itself to the person. (B) Only print. (C) Install and configure Caddy.
**Chosen: A**, as the owner decided; C reaches into the host.

## 3. Questions and flags

They come after the existing ones (the public and admin addresses are known by then) and follow the same rules as the
rest: with `--yes`, or without a terminal, every question takes its default; a flag answers a question. Secrets are never
a flag, only the environment variable the core itself reads, so the person who exports it for the installer exports what
the core wants.

| Question | Flag | Default |
|---|---|---|
| Send mail? (register and reset need it) | `--mail yes\|no` | no |
| Sender, as in `Vumlo <no-reply@example.edu>` | `--mail-from` | `Name <no-reply@host of public origin>` |
| SMTP host | `--mail-host` | required if mail is on |
| Encryption: starttls, tls, none | `--mail-tls` | starttls |
| Port | `--mail-port` | 587 for starttls, 465 for tls, 25 for none |
| User name (empty: no login) | `--mail-user` | empty |
| Password | env `VUMLO__MAIL__TRANSPORT__PASSWORD`, else asked (hidden) | empty |
| Files: local or S3 | `--files local\|s3` | local |
| S3 endpoint (empty: AWS) | `--s3-endpoint` | empty |
| S3 bucket | `--s3-bucket` | required if S3 |
| S3 region | `--s3-region` | `eu-north-1` (also with an endpoint: many S3-compatible services ignore it) |
| S3 keys | env `VUMLO__FILES__S3__ACCESS_KEY`, `VUMLO__FILES__S3__SECRET_KEY`, else asked (hidden) | required if S3 |
| Skip the checks of mail and S3 | `--skip-checks` | off |

Mail kind `file` is a development setting and is not offered. A mail question answered `no` writes nothing, as today.
The endpoint must be https (the core refuses anything else) and is checked before the network is touched.

Writes: `[mail]` and `[mail.transport]`, or `[files]` with `[files.s3]`, uncommented from the example the release ships
(a release whose example lacks the lines stops with a message naming it, as `patchToml` does today for `accent`). The
passwords and keys go to `.env` under the names above, uncommented where the example has them commented, never to
`vumlo.toml`, never printed.

## 4. The checks

Run once, after the questions and before any file is written or any service started; a failure stops the install with
the server's own words and removes what the run made.

- **SMTP.** Connect to host:port (implicit TLS for `tls`), read the greeting, `EHLO`; for `starttls` issue `STARTTLS` and
  upgrade, then `EHLO` again; with a user name, log in with `AUTH PLAIN` when advertised, else `AUTH LOGIN`, and expect
  success; `QUIT`. No message is sent. Timeout 10 s per step. A login that fails prints the server's reply line and never
  the password.
- **S3.** `HEAD` the bucket, SigV4-signed with the keys. Addressed the way the core's client does (`object_store`'s
  `AmazonS3Builder`): without an endpoint, `https://<bucket>.s3.<region>.amazonaws.com/`; with one, `<endpoint>/<bucket>`.
  200 passes; 403 reports "keys refused or no access"; 404 reports "no such bucket"; a redirect (301 with
  `x-amz-bucket-region`) reports the region the bucket is in.
- `--skip-checks` skips both, for a host the installer's machine cannot reach (it runs where the person types, not where
  Vumlo will run). Without a terminal nothing is asked; the flag is the only way past a failing check.

Risk named: Node's TLS trusts its own CA bundle, the core trusts what rustls trusts. A mail server with an internal CA can
pass in the core and fail in the installer. The failure message for a certificate error says so and names
`--skip-checks`. The opposite (passes here, fails in the core) is possible only for a host reachable differently from the
server; the doctor already reports mail and storage after the start.

## 5. Retry, marker, cleanup

- The marker is the file `.create-vumlo-unfinished` in the installation folder, written right after the unpack and
  removed as the last act of a successful run.
- Any failure before the services are started removes what this run created: the folder if it created it, otherwise the
  folder's contents (the folder was empty when the run began, so nothing else is lost). This also fixes today's gap
  with an existing empty folder.
- A failure during `--start` leaves the files and the marker, so the output can be read (as the README promises today).
- A folder that is not empty and has the marker is an unfinished installation: ask "Remove it and start over?" (default
  yes; `--yes` and no terminal take the default). Then, when `deploy/compose.yaml` is there, run
  `docker compose -f deploy/compose.yaml down -v --remove-orphans` in the folder (a failure of this step is a warning:
  Docker may be absent), then remove the folder's contents and install as usual. The volumes go with it, so a new
  generated database password cannot meet an old volume.
- A folder that is not empty and has no marker stops the run, as today. A finished installation never has the marker.
- Nothing is cached on disk by the installer (the release is unpacked straight from memory), so there is no archive to
  delete.

## 6. Proxy and the end of `--start`

- When `public_origin` is https, write `server.client_ip_header = "x-real-ip"` (uncomment the example's line). Only then:
  the header is trusted, so it is wrong to set it with no proxy that overwrites it, and the closing note says a proxy
  that does so is expected (the release's Caddyfile does).
- Write `caddy.env` in the installation folder with `VUMLO_DOMAIN=<host of public_origin>` and
  `VUMLO_ADMIN_DOMAIN=<host of admin_origin>`, for `sudo tee -a /etc/default/caddy < caddy.env`; the closing note prints that
  command and the copy of `deploy/Caddyfile`. Written only when both origins are https on a real hostname. No secrets in it.
- `--start`, after `doctor` and the plugin registration, runs `docker compose down` (no `-v`): the data stays, the
  services stop. The closing note says `docker compose up -d` starts them, and that the administrator and the address of
  the console are as printed. The marker is removed after the `down`.

## 7. Tests and docs

`test.mjs` (no network): a fake SMTP server on a local port (greeting, EHLO, AUTH with a right and a wrong password,
STARTTLS with a throwaway certificate when `openssl` exists, else that case is skipped and said) and a fake S3 server
(HEAD with a good signature, a wrong key, a missing bucket, a region redirect). The signature is checked against the
published SigV4 test vector. Also: `patchToml`/`patchEnv` for each new block against the example files of `../vumlo-core`;
the marker lifecycle with a fixture archive (failure before start removes the contents of an existing empty folder; a
marked folder is wiped and reinstalled; an unmarked one is refused; the marker is gone after success); `caddy.env`
only for https; `--start` ends with `down` (the `docker` call is a stub on `PATH`, as the existing `--start` tests have).
README: the table of questions, the checks, the retry and the end-of-start behaviour. CHANGELOG of `vumlo-core`:
none (the release does not change). `docs/install.md`: the manual Caddy steps note that the installer writes `caddy.env`.

## 8. Left to the builder

The exact wording of prompts and messages; the SMTP dialogue's handling of multi-line replies; how the hidden prompts
are done where `@clack/prompts` already offers one; where in `index.mjs` the checks live (a second file is fine if the
file grows past what one reads at a time). The version of the installer to publish is the owner's call.
