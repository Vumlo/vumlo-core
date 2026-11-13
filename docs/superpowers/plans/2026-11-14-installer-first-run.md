# The installer's first-run setup: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline) to implement this plan task by task.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** `npx create-vumlo` asks for mail, file storage and proxy settings, checks mail and S3 before anything starts, starts
over cleanly on a retry, and stops the services when `--start` is done.

**Architecture:** all work is in the `create-vumlo` repository (`/home/partum/Projects/vumlo/create-vumlo`): `index.mjs`
gets the questions, writers and the marker lifecycle; a new `checks.mjs` holds the SMTP and S3 checks (Node built-ins only);
`test.mjs` gets fake servers and fixtures. The release side changes only in docs. Branch `feat/first-run-setup`, cut from
`feat/admin-origin` (PR #4, unmerged, its `adminOrigin` code is needed).

**Tech Stack:** Node >= 20 ESM, `node:net`, `node:tls`, `node:http`, `node:crypto`, `@clack/prompts` (already a dependency).

**Spec:** `docs/superpowers/specs/2026-11-14-installer-first-run-design.md` (vumlo-core, branch `docs/installer-first-run-design`).

## Global Constraints

- No new dependency (spec section 2, choice A).
- Secrets are never a flag, never in `vumlo.toml`, never printed; they are read from `VUMLO__MAIL__TRANSPORT__PASSWORD`,
  `VUMLO__FILES__S3__ACCESS_KEY`, `VUMLO__FILES__S3__SECRET_KEY`, else asked with a hidden prompt (not when quiet), and
  written to `.env` under those names.
- With `--yes` or without a terminal every question takes its default; a flag answers a question (existing `ask` helper).
- S3 region default `eu-north-1`; S3 endpoint must be https; mail defaults to no; mail kind `file` is not offered.
- Marker file name: `.create-vumlo-unfinished`. Proxy file: `caddy.env` in the installation folder.
- Tests run without network; `docker` is a stub on `PATH` (see the existing `--start` tests in `test.mjs`).
- `package.json` `files` lists `index.mjs` only: `checks.mjs` must be added to it.
- Conventional Commits, small commits, no push (the owner pushes). `npm test` (= `node test.mjs`) must pass at each commit.

## Review Focus

- A failed run in an existing empty folder leaves it empty (today it keeps a half-unpacked release).
- A folder with real content and no marker is never wiped.
- A new generated database password cannot meet an old Postgres volume (`down -v` before the reinstall).
- A wrong SMTP login or S3 key fails before any file is written, and the secret is not in the output.
- `client_ip_header` is written only for an https public origin.
- `--start` ends with `docker compose down` and no `-v`.

---

### Task 1: Marker, cleanup and "start over"

**Files:**
- Modify: `create-vumlo/index.mjs` (the install part of `main`, lines ~711-725, ~895-901, ~903-921)
- Test: `create-vumlo/test.mjs`

**Interfaces:**
- Produces: `export const UNFINISHED = ".create-vumlo-unfinished"`; `export function removeContents(dir: string): void`
  (removes every entry of `dir`, keeps `dir`); `async function startOver(dir, { quiet }): Promise<void>` (not exported).

- [ ] **Step 1: Write the failing tests** in `test.mjs`, in the fixture section, using `cli()` and the `docker` stub of the
  `--start` block (a `docker` that logs `docker $*` to a file and fails `compose build` when told to):
  - `failed run in an existing empty folder`: `mkdirSync(d)`; `cli("--archive", v1, "--yes", "--dir", d, "--modules", "nope")`
    exits non-zero with `this release has no module "nope"`, and `readdirSync(d)` is `[]`.
  - `a folder with content and no marker is refused`: put `keep.txt` in a folder, run install, exit non-zero with
    `is not empty`, and `keep.txt` is still there.
  - `a failed --start leaves the marker, a retry starts over`: with the stub failing `compose build`, `--start` fails and
    `<dir>/.create-vumlo-unfinished` exists; add `<dir>/leftover.txt`; switch the stub to succeed and rerun the same command:
    exits 0, `leftover.txt` is gone, the marker is gone, and the stub log has `docker compose -f deploy/compose.yaml down -v
    --remove-orphans` before the first `docker compose build` of the second run.
  - `a finished install has no marker`: after any successful install `existsSync(marker)` is false.
- [ ] **Step 2: Run** `node test.mjs`. Expected: the new assertions FAIL (message "is not empty" test passes already; the
  others fail), earlier sections still pass.
- [ ] **Step 3: Implement.** In `main`: if `dir` is non-empty and holds the marker, `await startOver(dir, { quiet })`, else
  throw `is not empty` as today; compute `created` after that. `startOver` asks `p.confirm("... Remove it and start over?")`
  (default yes; quiet takes yes), runs `runIn(dir, ["docker","compose","-f","deploy/compose.yaml","down","-v","--remove-orphans"], { capture: true })`
  when `deploy/compose.yaml` exists and `hasDocker()`, warns (does not throw) on a non-zero status, then `removeContents(dir)`.
  After `unpack` write the marker. In the `catch` of the try: `created ? rmSync(dir, ...) : removeContents(dir)`. Remove the
  marker on the no-`--start` success path, and after `bringUp` returns on the `--start` path (Task 6 moves it after `down`).
- [ ] **Step 4: Run** `node test.mjs`. Expected: all sections print `ok`.
- [ ] **Step 5: Commit** `feat: a retry starts over; a failed run leaves nothing behind (ADR-0096)`.

---

### Task 2: Config writers for mail, files, proxy

**Files:**
- Modify: `create-vumlo/index.mjs` (`patchToml`, `patchEnv`, new helpers next to `defaultAdminOrigin`)
- Test: `create-vumlo/test.mjs`

**Interfaces:**
- Produces:
  - `export const defaultPort = (tls: "starttls"|"tls"|"none") => 587|465|25`
  - `export function mailSettings(a: { mail?: string, from?: string, host?: string, tls?: string, port?: string|number, user?: string }, name: string, origin: string): null | { from, host, tls, port: number, user: string }`
    — `null` when `a.mail` is not `"yes"`; throws with a plain message for: no host, `tls` not one of the three, port not 1..65535;
    `from` defaults to `${name} <no-reply@${hostname of origin}>` (with `<>"` stripped from `name`); `tls` defaults to `starttls`.
  - `export function filesSettings(a: { files?: string, endpoint?: string, bucket?: string, region?: string }): { storage: "local" } | { storage: "s3", endpoint: string|undefined, bucket: string, region: string }`
    — throws for `files` not `local|s3`, S3 without bucket, an endpoint that is not an `https://` origin; `region` defaults to `eu-north-1`.
  - `export function caddyEnv(origin: string, adminOrigin: string): string | null` — `VUMLO_DOMAIN=<host>\nVUMLO_ADMIN_DOMAIN=<host>\n`,
    `null` unless both origins are `https:` and neither host is `localhost` or an IP address.
  - `patchToml(text, { ..., mail?, files?, proxy? })`: `mail`/`files` are the results above; `proxy: boolean` uncomments
    `server.client_ip_header = "x-real-ip"`. `patchEnv(text, { ..., secrets? })`: `secrets: { NAME: value }`, each non-empty value
    set with the existing `setEnvVar` (a commented example line stays as it is; the line is appended).
- Consumes: `setEnvVar`, `patchToml`, `patchEnv`, `set` pattern of `patchToml` (throws when an example line is missing).

- [ ] **Step 1: Write the failing tests**, against `read("vumlo.example.toml")` and `read("env.example")` of `../vumlo-core/deploy/`:
  - mail on: output has `[mail]`, `from = "Example <no-reply@lms.example.edu>"`, `[mail.transport]`, `kind = "smtp"`, `host`, `port = 587`,
    `tls = "starttls"`, `username`; no `password` anywhere; `link_base` and the limits stay commented.
  - mail off (`null`): output equals the output without the parameter.
  - files s3 with an endpoint and without one: `storage = "s3"`, `[files.s3]`, `bucket`, `region = "eu-north-1"`, `endpoint` only when given.
  - `proxy: true`: `client_ip_header = "x-real-ip"` uncommented; `false`/absent: still commented.
  - `patchEnv` with `secrets`: `VUMLO__MAIL__TRANSPORT__PASSWORD=pw` present, empty values not written.
  - `mailSettings` throws on no host / `tls = "ssl"` / port `70000`; `filesSettings` throws on `http://` endpoint and on S3 without bucket;
    `caddyEnv` is `null` for `http://localhost:3000`, for `https://10.0.0.5`, and a string for `https://lms.example.edu` + `https://admin.lms.example.edu`.
  - Parsed with `toml`-free checks only (regex on lines); the real example files make a drifted release fail this test.
- [ ] **Step 2: Run** `node test.mjs`. Expected: FAIL (functions not exported).
- [ ] **Step 3: Implement** the functions above. For the example's commented blocks use one anchored line regex per key
  (`^# \[mail\]$`, `^# from = .*$`, `^# \[mail\.transport\]$`, `^# kind = "smtp".*$`, `^# host = .*$`, `^# port = 587.*$`, `^# tls = .*$`,
  `^# username = .*$`; `^storage = "local".*$`, `^# \[files\.s3\].*$`, `^# endpoint = .*$`, `^# bucket = .*$`, `^# region = .*$`;
  `^# client_ip_header = .*$`), through the existing `set`; an unset optional line (empty username, no endpoint) is removed with its newline.
- [ ] **Step 4: Run** `node test.mjs`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: write the mail, storage and proxy settings of an installation (ADR-0096)`.

---

### Task 3: The checks

**Files:**
- Create: `create-vumlo/checks.mjs`
- Modify: `create-vumlo/package.json` (`files`: add `checks.mjs`)
- Test: `create-vumlo/test.mjs`

**Interfaces:**
- Produces (all `async`, throw `Error` whose message is human, naming the server's reply, never the password or keys):
  - `export async function checkSmtp({ host, port, tls, user }, password: string, { timeout = 10000 } = {}): Promise<void>`
    — implicit TLS for `tls: "tls"`; greeting, `EHLO`, `STARTTLS` + upgrade + `EHLO` for `starttls`; with `user`, `AUTH PLAIN` when
    advertised else `AUTH LOGIN`, success is `235`; `QUIT`. A certificate error adds: "Node does not trust this certificate; if the server
    does, use --skip-checks".
  - `export async function checkS3({ endpoint, bucket, region }, { accessKey, secretKey }, { timeout = 10000 } = {}): Promise<void>`
    — signed `HEAD` (SigV4, service `s3`, payload hash of the empty body); URL `https://<bucket>.s3.<region>.amazonaws.com/`
    without an endpoint, `<endpoint>/<bucket>` with one; 200 passes; 403 "keys refused or no access"; 404 "no such bucket"; 301 with
    `x-amz-bucket-region` names that region.
  - `export function signV4({ method, url, region, service, accessKey, secretKey, date, payloadHash, headers = {} }): Record<string,string>` —
    the headers to send (`authorization`, `x-amz-date`, `x-amz-content-sha256`, `host`, plus `headers`); pure, for the vector test.
- `checkS3` does not itself require https (`filesSettings` does, before the network is touched), so tests point it at `http://127.0.0.1:<port>`.

- [ ] **Step 1: Write the failing tests**:
  - `signV4`: AWS's documented "GET Object" example (`GET /test.txt` on `examplebucket.s3.amazonaws.com`, `range: bytes=0-9`,
    `AKIAIOSFODNN7EXAMPLE` / `wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY`, region `us-east-1`, `20130524T000000Z`) gives
    `Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41`. If the builder's memory of the vector is unsure, look it up
    in the AWS docs (SigV4 header-based auth examples) before trusting a green test.
  - fake SMTP server (`node:net`, scripted replies, `AUTH PLAIN` right/wrong password, `none` TLS): `checkSmtp` passes with the right
    password; with the wrong one rejects with the server's `535` line and the message does not contain the password; refused connection
    rejects; a server that never answers rejects within `timeout`.
  - STARTTLS and implicit TLS with a throwaway certificate made by `openssl req -x509 -newkey rsa:2048 -nodes ...` into a temp dir,
    when `openssl` exists (else print `skipped (no openssl)` and continue): a self-signed certificate rejects with the "--skip-checks" hint.
  - fake S3 (`node:http`, endpoint `http://127.0.0.1:<port>`): 200 passes; 403, 404 and 301 each reject with the message above; the request carries
    an `authorization` header starting `AWS4-HMAC-SHA256 Credential=<accessKey>/`.
- [ ] **Step 2: Run** `node test.mjs`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement** `checks.mjs`. Multi-line SMTP replies (`250-` continues, `250 ` ends). One `timeout` per step.
- [ ] **Step 4: Run** `node test.mjs`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: check the SMTP login and the bucket before anything starts (ADR-0096)`.

---

### Task 4: Questions, flags and wiring

**Files:**
- Modify: `create-vumlo/index.mjs` (`parseArgs` options, `main`, the closing notes)
- Test: `create-vumlo/test.mjs` (fixture release, `cli()`)

**Interfaces:**
- Consumes: `mailSettings`, `filesSettings`, `caddyEnv`, `patchToml`, `patchEnv`, `checkSmtp`, `checkS3`, `ask`, `ok`, `p.password`.
- New options: `mail`, `mail-from`, `mail-host`, `mail-tls`, `mail-port`, `mail-user`, `files`, `s3-endpoint`, `s3-bucket`, `s3-region`,
  `skip-checks` (boolean). Questions follow the existing ones (after the admin-origin question), in the order of the spec's table;
  the password and keys: environment variable first, else `p.password` when not quiet, else empty.
- Order inside `main`: questions, then the checks (unless `--skip-checks`; the message `Checking the mail server` / `Checking the bucket`
  on the spinner), then the file writes. A failing check throws, and the existing `catch` removes what the run made.

- [ ] **Step 1: Write the failing tests.** Extend the `release()` fixture's `vumlo.example.toml` text with the real `[files]`,
  `# client_ip_header` and `[mail]` lines of `../vumlo-core/deploy/vumlo.example.toml`, and the fixture `env.example` with
  `# VUMLO__MAIL__TRANSPORT__PASSWORD=`. Then, through `cli()` with a fake SMTP server on a local port (Task 3's helper):
  - mail on with `--mail-tls none --mail-host 127.0.0.1 --mail-port <p> --mail-user u` and the password in the environment:
    exit 0; `vumlo.toml` has the `[mail]` blocks; `.env` has `VUMLO__MAIL__TRANSPORT__PASSWORD=<pw>`; the password is in neither `vumlo.toml` nor the output.
  - same with a wrong password: non-zero, the output has the `535` line, no `vumlo.toml` exists and the folder is gone (it was created by the run).
  - `--skip-checks` with an unreachable host: exit 0.
  - S3 flags with `--skip-checks` (the check itself is Task 3's): `storage = "s3"` and `[files.s3]` written; keys in `.env` only, never in the toml or the output.
  - `--origin https://lms.example.edu --admin-origin https://admin.lms.example.edu`: `client_ip_header = "x-real-ip"` in the toml and
    `caddy.env` with the two domains; with `http://localhost:3000`: neither.
  - no mail/files flags: `vumlo.toml` has no `[mail]` and `storage = "local"` (nothing changes for an existing script).
- [ ] **Step 2: Run** `node test.mjs`. Expected: FAIL.
- [ ] **Step 3: Implement.** The closing note (both `Next` and `Running`) prints, when `caddy.env` was written:
  `sudo tee -a /etc/default/caddy < caddy.env`, `sudo cp deploy/Caddyfile /etc/caddy/Caddyfile`, and that a proxy that overwrites `X-Real-IP` is expected.
- [ ] **Step 4: Run** `node test.mjs`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: ask for mail and file storage, write the proxy settings (ADR-0096)`.

---

### Task 5: `--start` ends with the services stopped

**Files:**
- Modify: `create-vumlo/index.mjs` (`bringUp`, the end of `main`)
- Test: `create-vumlo/test.mjs` (the `--start` block)

- [ ] **Step 1: Write the failing tests** in the `--start` block: after a successful `--start` run the stub log's last compose call
  is `docker compose down` (no `-v`, no `--remove-orphans`), after the plugin restart; the marker file is gone after it; the closing
  note says `docker compose up -d`; a `--start` that fails midway still leaves the marker and does not call `down`.
- [ ] **Step 2: Run** `node test.mjs`. Expected: FAIL.
- [ ] **Step 3: Implement.** At the end of `bringUp`, `step(dir, "Stopping the services (the data stays)", ["docker","compose","down"])`
  with the plugins' compose file in play (`runIn` already adds it); then the caller removes the marker (move it from Task 1's
  position). The `Running` note becomes `Installed. Start it with: docker compose up -d` plus the existing lines; its outro
  `Vumlo is installed and stopped.`; update the existing assertion on `Vumlo is running`.
- [ ] **Step 4: Run** `node test.mjs`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: --start stops the services when it is done (ADR-0096)`.

---

### Task 6: Docs

**Files:**
- Modify: `create-vumlo/README.md`; `vumlo-core/docs/install.md`, `vumlo-core/docs/decisions/in-progress.md` (branch `docs/installer-first-run-design`)

- [ ] **Step 1: README**: the table of questions gets the mail, files and `--skip-checks` rows; sections for the checks, the retry/marker
  and `caddy.env`; "Start it in one go" says it ends with the services stopped; the `Development` section lists `checks.mjs`.
- [ ] **Step 2: vumlo-core**: `install.md`'s Caddy section notes that `npx create-vumlo` writes `caddy.env` for `/etc/default/caddy`;
  `in-progress.md` gets the ADR-0096 line (installer asks mail, storage, proxy; checks before start; marker retry; `--start` stops).
- [ ] **Step 3: Run** `node test.mjs` in create-vumlo. Expected: PASS.
- [ ] **Step 4: Commit** each repo: `docs: the installer's first-run questions (ADR-0096)`.
