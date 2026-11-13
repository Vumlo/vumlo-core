# Changelog

All notable changes to Vumlo are in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version is the one in
`backend/Cargo.toml` (`[workspace.package] version`), which the server, `vumlo-cli` and the web app share.

Before 1.0, a new minor version (`0.2.0`) may change the configuration, the API or the database
migrations; a patch version (`0.1.1`) only fixes things. Every release has **Upgrade notes**: what you
must do, in order, besides getting the new release. See [RELEASING.md](RELEASING.md).

## [Unreleased]

### Added

- **Org units and unit roles** (ADR-0094, backend; the web pages come next). An admin builds a tree of units (`/units`) and makes a teacher a `manager` or a `viewer` of one (`/units/{id}/roles`); the role covers the unit and every unit below it, so a dean sees and runs the courses of their faculty without being an admin or being added to each course. A course sits in at most one unit (`unit_id` on `POST /courses` and `POST /templates/{id}/courses`, `PUT /courses/{id}/unit`); `GET /units/{id}/courses` lists them. What a role may do is a set of switches the admin changes at any time and that works on the next request (`GET` and `PUT /admin/unit-roles`): `view_grades`, `edit_content`, `grade`, `manage_roster`, `manage_courses`, `manage_units`; by default a manager has all but `edit_content` and `grade`, a viewer has `view_grades`. A unit role never deletes a course, transfers its ownership or acts as a student. `GET /courses/{id}` carries `can`, what the caller may do in the course, and `unit_id`. A change made in a course through an admin's or a unit's access is written to the audit log with `meta.via` (`admin` or `unit`). A teacher who becomes a student loses their unit roles.
- **Synchronous event subscribers and `content.facts`** (ADR-0093). `events::emit` also runs the subscribers the process installed, inside the transaction of the change. `content` keeps its own copy of what a student did (the first submitted attempt, the published score, their mark), so the access rules of ADR-0092 no longer read `assessment` or `gradebook`; the quiz's draft grade is written by a subscriber too. A new internal event `grade.publication_changed` (never sent to plugins). `vumlo-cli facts rebuild [--course ID]` makes the facts again from the submissions and grades. No change to the API.
- **A target, a pass mark and groups in the gradebook** (ADR-0089). A course has a `target` (default 100) and an optional `pass_mark`; its assignments sit in groups that can have a cap, count only the best K marks, count as practice (graded, not in the total) or as bonus. The total is `min(target, sum of the groups)`. A course shows its groups as its top-level modules (`course`, the default) or as groups of the teacher's own (`custom`, with "Not sorted" for what has no group yet), and can switch between them without losing anything. New: `GET /courses/{id}/gradebook/structure`, `PUT /courses/{id}/gradebook/settings`, `POST /courses/{id}/gradebook/groups`, `PUT|DELETE /courses/{id}/gradebook/groups/{key}`, `PUT /courses/{id}/gradebook/placement` and `GET /courses/{id}/my-grade-summary`; the staff gradebook page now carries the layout and a total for every student.

### Changed

- **`learning` is five crates** (ADR-0093). `teaching`, `content`, `assessment` and `gradebook` are crates of their own under `backend/crates/learning/`, and `learning` (in `learning/flows`) holds only the flows, the scheduler and the audit report. Cargo enforces `teaching < content < assessment < gradebook < learning` (most wrong edges are a cycle; `architecture.rs` checks the manifests for the rest). Each crate has its own state, error enum and router; `server` builds the four states beside `LearningState` and merges the routers as before. No change to the API, the database or the configuration; an installation built from the source archive needs nothing new. Anything that named `learning::content`, `learning::teaching`, `learning::assessment` or `learning::gradebook` (a module of the optional `modules/*`, a plugin host) names the crate instead.
- **Inside `learning`, nothing in a lower module reads or calls a higher one** (ADR-0093). The four modules are stacked `teaching < content < assessment < gradebook`; what a lower one needed from a higher one is now a flow (`learning::courses`: cards, header, people, publish, delete, templates; `learning::items`: create, change or delete an item or a module, read a form, the result of a quiz; `learning::submissions`: save a draft, submit, auto-submit) or a read model that sits in `gradebook` (`/me/work`, the facts of the items, the figures of the course cards). `architecture.rs` enforces the order. The reads that go down are views the owner declares (migration `0054`; an item's body, a submission's content and a grade's drafts are not in them), so a module no longer reads another's tables. The check that a file may be named by an item or a submission is part of those flows and locks the file rows, so two saves that name one upload can no longer both take it, and the orphan sweep cannot delete a file that is being attached. A request that is wrong in two ways (a file the caller may not use and, say, a bad config) now reports the validation error first and the file second, and when a change names several files, one the caller may not use is reported before an image block that names a file that is not an image. No change to the API.
- **The regular assignments of a course on the `hundred` scale may add up to more than 100 points** (ADR-0089 replaces that part of ADR-0022). The total is cut at the target instead. The twelve-point limit on one assignment stays.

**Upgrade notes.** Run `db migrate`: migration `0048_gradebook_groups.sql` adds three tables to the schema `gradebook`; nothing existing is changed. Migrations `0051` to `0053` (ADR-0093) create `content.facts` and fill it from the submissions, grades and marks that exist, drop `content.completions`, and add `assessment.submissions.auto_total`. Migration `0054` adds eleven read views between the modules of `learning` (`content.item_list`, `gradebook.published_grades`, ...) and changes no table or row. Migration `0055_org_units.sql` (ADR-0094) adds the tables `org.units`, `org.unit_roles` and `org.unit_role_abilities` (seeded with the default abilities), the views `org.unit_subtree` and `org.unit_grants`, and a nullable `unit_id` on `teaching.courses`; no course is in a unit and nobody holds a unit role until an admin sets them. Nothing else has to be done; `vumlo-cli facts rebuild` repairs the facts if they are ever wrong.

## [0.6.1] - 2026-11-11

### Fixed

- **The Windows installer of the desktop app opened a blank "file not found" page** (`ERR_FILE_NOT_FOUND`). `desktop/scripts/build.mjs` gave Tauri the web build as an absolute path, and Tauri reads a string that can be a URL as one: on Windows `D:\a\web\out` is the URL `d:\a\web\out`, so nothing was embedded and the window opened that "address". The path is now relative to `desktop/` with slashes, as in `tauri.conf.json`, and the configuration refuses a value that would be read as a URL. Linux and macOS installers were not affected.

**Upgrade notes.** Nothing to change on the server: no migration, no configuration key, and the server, the web app and `vumlo-cli` are unchanged. Only the desktop app is: build a new installer (`node scripts/build.mjs`, or the `desktop-v*` workflow) for Windows and publish its address again; people install it over the old one.

## [0.6.0] - 2026-11-11

### Added

- **A mail layer** (ADR-0084). All outgoing mail is a row of the new schema `mail`, one per recipient, queued in the transaction of whatever asked for it and sent by a mail worker (in `serve` and in `worker`) within a few seconds. A transient failure is retried with a growing delay; a permanent one, or a message a day old, is a `dead` row. The address is copied into the row and erased `mail.retention_days` (30) after the message ends, at once for account mail. Verification and password-reset mail go through it with the same text as before. `[mail]` gained `plugin_hourly_calls` (500), `recipient_daily` (20), `max_audience` (2000), `retention_days` (30) and `journal_days` (180).
- **Plugins can send mail** (ADR-0085). A new scope `mail:send` and `POST /plugin-api/mail`: one person (who must be on the roster of a course the plugin reaches) or a course audience (`students`, `teachers` with assistants, `all`), plain text, with a `kind` for opt-out, an optional `link_path` and `dedupe_key`. The plugin never sees an address. Only active people with a verified address are mailed; the answer counts who was skipped and why. Limits answer `429` with `Retry-After`; an audience over `mail.max_audience` is `422`; without `[mail]` the route is `503`. Each accepted call is in the audit log (ids and counts, never text), and disabling or removing a plugin cancels its queued mail.
- **One-click opt-out** (ADR-0086). Every plugin mail has a footer naming the plugin and a link that turns off that kind of mail from that plugin for that person (`GET|POST /api/v1/mail/opt-out/{token}`, no sign-in; `…/opt-in/{token}` undoes it), and the `List-Unsubscribe` headers of RFC 8058. Account mail has no opt-out.
- **`vumlo-cli mail status|list|retry`** (ADR-0087): the queue and the delivery journal, by id and never with an address, and a way to put dead messages back (`mail retry <id> | --dead [--plugin ID] [--dry-run]`, audited).
- **A "Plugin reports" panel in the web app** (ADR-0088). Teachers and admins see what plugins reported (ADR-0077) under a submitted attempt in the grading view, on the assignment page and on the course page. A report shows its summary and, when the plugin follows the display convention (`data.display`: `level`, `title`, a table of up to 200 rows, up to 20 facts, `links`), a badge, a title, an accessible table that scrolls on a narrow screen, figures and download buttons. Everything is shown as text; a link is followed only if it points into the same plugin's routes (`/api/v1/plugins/<id>/...`); numbers use the viewer's language; a report without a valid `display` shows the summary and its data folded away. Students never see the panel and their browser does not ask. English and Ukrainian.

**Upgrade notes.** Rebuild the web image (the reports panel is in the web app). Run `db migrate`: migration `0047_mail.sql` adds the schema `mail` and a column to the view `auth.people`. There is no down migration, so to go back restore the backup taken before it (an older binary refuses a database that is ahead of it). Nothing in `vumlo.toml` has to change; the new `[mail]` keys have defaults. A plugin that was granted nothing new cannot send mail: grant `mail:send` with `vumlo-cli plugin grant`. Verification and reset mail are now sent by the mail worker, which `serve` and `worker` start whenever `[mail]` is set; a failed send is a `dead` row in `vumlo-cli mail list` instead of a dead letter of the outbox.

## [0.5.0] - 2026-11-10

### Added

- **`enrollment.changed` follows a person's role and status** (ADR-0082). When an administrator changes a user's role or disables or enables them, a plugin subscribed to it is sent one event for each course whose roster that touches, as it already was for a change of a course membership. Before, it had to read the roster again to find out.
- **A plugin can ask what was deleted** (ADR-0081). `GET /plugin-api/assignments/deleted` and `GET /plugin-api/announcements/deleted` (`deleted_since`, `course_id`, `cursor`, `limit`) list the ids of published assignments and of announcements that were deleted, with when, for the courses the plugin reaches. They are kept `plugins.tombstone_days` days (30); asking about an earlier time is `410` and the plugin reads the list in full. Submitted attempts and grades cannot be deleted and the roster is read in full, so they have no such list.
- **Public routes of a plugin** (ADR-0080). A route in a plugin's manifest may declare `"auth": "none"`: anyone may call it at `/api/v1/plugins/{id}/{path}` with no session, for something like an iCalendar feed that an app fetches with a URL. A public route is `GET` only and has no roles or course. The core takes the call from an allowance kept for that route (`plugins.public_route_burst`, `plugins.public_route_requests_per_second`; over it is `429`), drops cookies as it always does, passes on only the types of answer it already passes on, and forwards the call with `X-Vumlo-Anonymous: 1` and a signature that says nobody signed in. The plugin checks its own token. `examples/hello-plugin` serves a feed this way.
- **Running a plugin that is in use** (ADR-0079). `vumlo-cli plugin upgrade` takes a plugin's new manifest and asks only about the scopes it did not ask for before; `plugin rotate-token` gives it a new token and cuts off the old one at once; `plugin calls` shows what it has called. Each plugin token has an allowance of calls to the plugin API (`plugins.api_burst`, `plugins.api_requests_per_second`; over it is `429` with `Retry-After`), every call leaves a line in a log kept `plugins.call_log_days` days, and what a plugin changes is in the audit log with the source `plugin`.
- **Pages and tools of a plugin behind the core's sign-in** (ADR-0078). A plugin's manifest may declare `routes`, each with the roles that may call it (institution roles, and roles in the course of a `{course_id}` in the path). People call them at `/api/v1/plugins/{id}/{path}` with their ordinary session; the core checks who they are and forwards the call with signed `X-Vumlo-*` headers (user, role, course, time, a nonce, a signature) and no cookie. Only JSON, text, CSV, iCalendar, PDF and XLSX come back to the browser. `examples/hello-plugin` declares two routes and checks the signature, the time and the nonce.
- **Plugin reports on assignments and courses, under keys** (ADR-0077). A plugin with `reports:write` stores a report on a submitted attempt, an assignment or a course with `PUT /plugin-api/{submissions|assignments|courses}/{id}/reports/{key}`, replaces it by writing again and deletes it with `DELETE`; staff read them at `GET /assignments/{id}/plugin-reports` and `GET /courses/{id}/plugin-reports` besides the existing one for an attempt, and each report now says its `key`.
- **Events for plugins, and course announcements** (ADR-0075, ADR-0076). A plugin can subscribe to `assignment.created`, `assignment.updated`, `assignment.deadline_changed`, `enrollment.changed`, `quiz.attempt_submitted` and `announcement.published`; every delivery now carries `occurred_at`. Teachers and admins publish announcements at `POST /courses/{id}/announcements` (members read them; there is no screen in the web app yet), and a plugin with the new scope `announcements:read` reads them.
- **Plugins read the course, within the courses they are granted** (ADR-0074). New scopes: `assignments:read` (assignments and the field definitions of a form, the values of a submitted form), `roster:read` (people of a course with their roles, no contact data), `grades:read` (published grades only), `quizzes:read` (the questions of a quiz without its key; attempts with the score per question), `users:contact` (the address of people in a granted course). `submissions:read` gains a list per assignment (when each attempt was submitted and graded), and a submission now carries `course_id`. Lists page with `cursor` and `limit` and catch up with `updated_since`. An administrator grants a plugin `all` courses or a list (`vumlo-cli plugin install|grant --courses`); every call is checked against it, and a plugin is sent only the events of its courses. `examples/hello-plugin` reads the type of the assignment.

- **Plugin settings** (ADR-0072). A plugin's manifest may declare `settings` (string, number, boolean, `enum`, list; `default`, `description`, `secret`, scope `institution` or `course`). Administrators manage them with `vumlo-cli plugin settings list|get|set|unset`; secrets are stored sealed (XChaCha20-Poly1305 from the `chacha20poly1305` crate, ADR-0073; the key is derived from `plugins.secret_key`) and never shown. A plugin with the new scope `settings:read` reads its values at `GET /api/v1/plugin-api/settings` (`404` for a `course_id` that names no course), and is sent `plugin.settings.changed` when one changes. `examples/hello-plugin` uses them.

- **The accent colour is back** (ADR-0083, which supersedes the removal in ADR-0054). `institution.accent` is one of `violet` (the default, and exactly what the web app looked like before), `blue`, `teal`, `green`, `orange` or `red`. The whole palette follows one hue (`--hue` in `web/src/styles/tokens.css`), the root element carries `data-accent`, and the key is public, so it is in `client-config.json`.
- **The release archive carries the optional modules** (ADR-0083). `backend/crates/modules/ai_agent` is in `vumlo-<version>-source.tar.gz` (only the test-only `dummy` is left out), so an installation can build it with `--features ai_agent` and `npx create-vumlo` can offer it.
- **A plugin in a container is reachable over `http://`** (ADR-0083). The base URL of a plugin (`vumlo-cli plugin install <url>`) accepts `http://` for a host with a single label, such as `http://plugin-text-metrics:8000`, the name of a container beside the core in Docker Compose. A name without a dot cannot be a host on the public internet, so nothing leaves the network the core runs in; a dotted name or an IP address other than loopback still needs `https://`. The `ai_agent` endpoint is not relaxed.
- **`./vumlo` and `deploy/update.sh --docker` read `plugins/compose.yaml`** when it exists, besides `deploy/compose.yaml`: the plugin containers `npx create-vumlo` made are started and rebuilt with the rest.
- **`vumlo-cli config schema`** prints the JSON Schema of `vumlo.toml`.
- **`npx create-vumlo update`** updates an installation in place (run it inside the installation): it backs up the files it replaces into `backups/` and never touches `vumlo.toml`, `.env`, `backups/` or `plugins/`. It replaces the side-by-side `vumlo-0.x.y/` folders of the manual upgrade in `docs/install.md`.

### Changed

- **`brand apply` writes into the web app** (ADR-0083). The client configuration goes to `web/src/generated/` and the manifest, favicon and icons (and `icon-1024.png`, which the desktop installers are made from) to `web/public/brand/`; `scripts/brand.mjs` only reads them. There is no `.vumlo/` folder any more, no `VUMLO_GENERATED_DIR` and no `--generated`. In an installation without a web app (`web.dir` has no `package.json`) `brand apply` says there is nothing to do and exits 0. `vumlo.schema.json` is not written; use `vumlo-cli config schema`.

**Upgrade notes.** **Delete the old `.vumlo/` folder** (it is not read any more) and run `./vumlo brand apply`, then rebuild the web image: the branding is now in `web/`. If you copied `desktop/release/desktop-release.yml` into your own repository, copy the new one (it reads `web/src/generated` and `web/public/brand`); a script that called `desktop/scripts/build.mjs --generated <dir>` calls it without the flag. The accent is optional: add `accent = "teal"` (or another name) under `[institution]` and run `brand apply` again. **A plugin that checks the signature of a call through the core (ADR-0078) must add a line to the text it signs: `session` (or `none` for a public route) after the request-target, and `-` for the user and the role when it is `none`; see `examples/hello-plugin/plugin.py`.** Run `vumlo-cli db migrate` (migrations 0039 to 0046). Plugins that are installed keep reaching every course (`all_courses` is set for them); narrow one with `vumlo-cli plugin grant <id> <scopes> --courses <ids>`. `vumlo-cli plugin install` now needs `--courses` (it asks when the flag is missing, and there is no default). **A plugin that reported with `PUT /plugin-api/submissions/{id}/report` must use `PUT .../reports/{key}`** (the reports it has written are kept, under the key `default`). Plugins that were installed keep working and declare no settings; to use settings, a plugin's new manifest must be read again (`plugin remove`, then `plugin install`), and the key `plugins.secret_key` must stay the same for as long as secret settings are in use.

## [0.4.1] - 2026-11-01

Branding, and a menu for the file types of a file assignment.

- File assignments: the accepted types are picked from a menu of groups, with a field for other types. `accept` entries may be one character long (`.c`, `.h`) (ADR-0023, amended).
- **Branding.** The logo is always Vumlo's: the favicon, the app icons, the navbar and the sign-in page. `institution.logo` is removed (ADR-0054, amended), and `resvg` with it.
- The name of the app is "Vumlo | Short name" on the web and "Vumlo Short name" on the desktop (the installers cannot have a `|` in their name).
- The title of a browser tab says where you are: "Lab report · Course · Vumlo | Short name".
- The sign-in page has Google's logo on its button and a proper language icon.

**Upgrade notes.** This release removes a configuration key, which a patch release should not. If your `vumlo.toml` has `logo = ...` in `[institution]`, delete that line: `config check` and the server stop with "institution.logo: unknown key". Then run `./vumlo brand apply` and rebuild the images. No migrations, no new `config_version`.

## [0.4.0] - 2026-11-01

File uploads (ADR-0069).

### Added

- Files in the editor and in assignments: `image` and `file` blocks in documents, the item kind `file`, and the
  assignment type `file` (a hand-in of up to `max_files` files, graded by hand). A teacher can preview a hand-in
  while grading.
- `POST /api/v1/files` (the raw file as the body) and `GET /api/v1/files/{id}`; a file is served only through an
  item or a submission the caller may read (otherwise `404`), with `Range` and `ETag`; it is `inline` only for
  PDF, PNG, JPEG, GIF, WebP and plain text, always with `X-Content-Type-Options: nosniff`, and with
  `Content-Security-Policy: sandbox` for every type except PDF (Chrome and Edge do not show their PDF viewer in a
  sandboxed response; the type is verified by the file's signature at upload, ADR-0069).
- Storage in a directory on the server or in an S3-compatible bucket (`[files]` in `vumlo.toml`). A file that no
  item and no submission names is deleted after 24 hours; the sweep runs hourly.
- Desktop app: upload a file, and "open" a file, which saves it in Downloads. Documents, spreadsheets,
  presentations, text, images, audio and video are opened; archives, programs, scripts and every other kind are
  only shown in their folder. Windows and macOS mark the file as downloaded (not run on those systems in testing).

### Changed

- The crate chain gained `files` between `auth` and `events` (ADR-0065, ADR-0069).
- A document that holds only an `image` or a `file` block is no longer counted as empty.
- An answer option of a quiz may hold an image as its one block.

### Upgrade notes

- `config_version` is still **1**.
- The newest core migration is `0038` (`0036` to `0038` are new: the `files` table, the tables that record which
  files an item or a submission names, and the item kind `file`). Take a backup, then run `vumlo db migrate`
  (`deploy/update.sh` does both). **A server older than this release cannot run against a migrated database.**
- New optional section `[files]`: `storage = "local"` (default) or `"s3"`, `path` (default `files`, relative to
  the directory of `vumlo.toml`), `max_size` (default 52428800, 50 MiB; at most 1073741824). With `s3`, add
  `[files.s3]` (`bucket`, `region`, optional `endpoint`) and set `VUMLO__FILES__S3__ACCESS_KEY` and
  `VUMLO__FILES__S3__SECRET_KEY` in the environment.
- **The server creates the local files directory when it starts and refuses to start if it cannot.** The shipped
  files now point it at a writable place with `VUMLO__FILES__PATH` (which overrides `files.path`). **systemd:**
  copy the new `deploy/systemd/vumlo-core.service` and `vumlo-worker.service` over the installed units (they add
  `StateDirectory=vumlo`, which creates `/var/lib/vumlo`, and `VUMLO__FILES__PATH=/var/lib/vumlo/files`; keep
  your own edits, such as paths), run `systemctl daemon-reload`, then `deploy/update.sh --systemd`; `update.sh`
  does not copy units. **Docker:** `deploy/compose.yaml` gets the `files-data` volume on `core` and `worker`
  and the image creates its mount point, so rebuild with `deploy/update.sh --docker`. Nothing to move: no
  uploaded files exist before this release. See [Uploaded files](docs/install.md#uploaded-files).
- A reverse proxy in front of the API must allow request bodies of `files.max_size`. The Caddyfile of the release
  already does; with nginx set `client_max_body_size`.
- **`vumlo db backup` does not contain the files.** Back up the files directory or the bucket separately and restore
  it together with the database.
- An upload is held in memory while it is stored: plan `files.max_size` of memory per upload in progress.
- Desktop: build a new app to get file upload and open; an app built against an older release does not have them.

## [0.3.0] - 2026-10-28

Plugins and optional modules, a redesigned web app, and a page for grading.

### Upgrade notes

- `config_version` is still **1**.
- The newest core migration is `0035`. Take a backup, then run `vumlo db migrate` (`deploy/update.sh` does both). The
  migrations rename the Postgres schema `identity` to `auth` (`0032`), add five read-only views (`0033`) and the schema
  `plugins` (`0034`, `0035`). **A server older than this release cannot run against a migrated database**, and anything of
  your own that reads the database directly (a report, a backup script that dumps `--schema=identity`) must say `auth`.
- To install plugins set `VUMLO__PLUGINS__SECRET_KEY` (32 characters or more) in the environment; without it none can be
  installed. Nothing else changes if you use none.
- The web app has a new look and a new page for grading; there is nothing to configure. Run `vumlo-cli brand apply` and
  rebuild the web image as always.
- The web image no longer serves `mockServiceWorker.js` or `mock-plugin/`, and the build fails if the browser bundle carries
  the mock API's sample data. They were for `npm run dev:mock` only.

### Added

- **Plugins** ([ADR-0066](docs/decisions/done.md)): external HTTP services in any language that the core
  does not trust. `vumlo-cli plugin install|list|grant|disable|enable|remove`; the core sends the events a plugin subscribed to
  (`submission.submitted`, `grade.published`), signed with `X-Vumlo-Timestamp` and `X-Vumlo-Signature`, and the plugin calls
  `/api/v1/plugin-api/*` with its token, where every call needs a scope (`submissions:read`, `reports:write`). Staff read what
  plugins reported at `GET /api/v1/submissions/{id}/plugin-reports`. An example is in `examples/hello-plugin/`.
  - **Upgrade notes:** run `vumlo db migrate` (migrations `0034_plugins.sql` and `0035_reports_follow_their_submission.sql` add the
    schema `plugins`). To use plugins set
    `VUMLO__PLUGINS__SECRET_KEY` (32 characters or more) in the environment; without it none can be installed.
- **A plugin can show a page in a tab of an assignment** (ADR-0066). The page is a sandboxed frame with no session and no access
  to the web app; it learns who is looking from a signed token the core issues for a minute (`POST /assignments/{id}/plugin-launch`)
  and draws with the theme and the design tokens the web app sends it. `GET /api/v1/plugin-slots?slot=assignment.tab` lists the
  pages. The tab is on text assignments and on assignments that take no submission, not yet on quizzes and forms, and the desktop
  app shows none. `deploy/Caddyfile` has an optional `frame-src` line for the origins of your plugins.
- **`GET /api/v1/meta`** lists the enabled optional modules and their versions.
- **A page for grading** a student's work: a cell of the gradebook opens it (the drawer is gone). It shows the submission, a
  rail of students with a mark for where each one stands, previous and next, a filter (to grade, drafts, all), and the grade
  panel with points, a comment, save, publish and the history. "Save and next" moves on; so does "Publish", which is off
  while there is nothing to publish. Enter in the points field saves and moves on. The confirmation is a toast, and a saved
  draft can be put back with Undo (a published grade cannot: students have seen it).
- **The gradebook can be filtered** by assignment, by status and by late work.

### Changed

- **The web app is redesigned** in a new visual system, and the teacher's course screens are recomposed. Formulas are drawn
  by KaTeX ([ADR-0068](docs/decisions/done.md)).
- **Optional modules are a `Module` trait** ([ADR-0066](docs/decisions/done.md)): a manifest (id, version,
  `requires`, permissions, settings), migrations, and `register` for routes, event handlers and background jobs; they start in
  the order of their `requires`. Module routes moved to `/api/v1/modules/{id}/...` and sign in like the core's, with permissions
  decided by `access`. A module's tables are in the schema `mod_<id>` with its history in `mod_<id>._sqlx_migrations`.
  `vumlo-config` no longer lists modules, and a name in `[modules]` that the binary was not built with is a configuration error
  whether it is on or off. Several event handlers may listen to one event type. The only module in the tree is still the
  test-only `dummy`, so a release installation has nothing to migrate; a fork with its own module has to port it to the trait.

- **The backend is one crate per layer** ([ADR-0065](docs/decisions/done.md)): `server` → `learning` → `org`
  → `auth` → `events` → `access` → `common`, so the compiler refuses a call from a lower layer to a higher one. The HTTP API,
  its OpenAPI document, the configuration and the commands are unchanged. For a build from source: the library crate `server`
  no longer holds the modules (`server::identity` is the crate `auth`, `server::policy` is `access`), and the database harness
  is the crate `test-support`, which the release archive leaves out.
  - **Upgrade notes:** run `vumlo db migrate`: migration `0033_read_views.sql` adds five read-only views
    (`auth.people`, `org.term_list`, `org.group_list`, `org.subgroup_list`, `events.audit_entries`) next to the rename of
    the next entry. Nothing else to do.
- **The Postgres schema `identity` is now `auth`** (migration `0032_auth_schema.sql`, `ALTER SCHEMA ... RENAME`), and the
  `identity` module of the server is the `auth` module. `vumlo-cli modules list` names `auth` and `access` (it named
  `identity` and `policy`). The HTTP API is unchanged.
  - **Upgrade notes:** run `vumlo db migrate` as always; rows, keys and indexes are kept. Anything of your own that reads the
    database directly (a report, a backup script that dumps `--schema=identity`) must say `auth`. A server older than this
    release cannot run against a migrated database.

## [0.2.2] - 2026-10-20

The names of the desktop installers, and of the app.

### Fixed

- **A login at the moment an administrator resets that user's password could fail with 500** (a deadlock in the database: the
  login held the user while it waited for the password, the reset held the password and wanted the user). The login now
  locks them in the order the reset writes them. Nothing to do on upgrade.
- **The installers of the desktop app are named after the URL scheme**, `vumlo-ucu_0.2.2_amd64.deb`, `..._x64-setup.exe`,
  `..._aarch64.dmg`, `..._x64.dmg`, `..._amd64.AppImage`, and not after the institution's name. A name with spaces or
  Cyrillic broke the checksum step of `desktop-release.yml`, so no release was made. `scripts/build.mjs` copies them to
  `desktop/dist/`.
- **A `vumlo-ucu://` link reaches the desktop app on Linux.** The `.deb`'s desktop file had `Exec=vumlo-ucu` without `%u`, so
  "Open app" at the end of a sign-in with Google started the app but never gave it the code. (The AppImage is built by
  Tauri with its own desktop file, which cannot be replaced: for Google sign-in on Linux use the `.deb`; password sign-in works in both.)
- **The desktop app is called by the short name** (`institution.short_name`, `UCU`) in the shortcut and the installer's
  metadata, not by the full name; the full name is the publisher. (A name in another alphabet made a `.deb` that dpkg
  refuses, "invalid package name"; the app is then called by its URL scheme.) **Upgrade notes:** nothing to change in `vumlo.toml`. If you copied `desktop-release.yml` into your
  repository, copy it again from `desktop/release/`.

## [0.2.1] - 2026-10-20

The Vumlo logo.

### Changed

- **The Vumlo logo** is the placeholder logo (`brand/logo.png`, in place of `brand/logo.svg`), and the web app shows an
  institution's logo as it is, in the header and on the sign-in pages, instead of the first letter of its name in a
  coloured tile. An institution with no `institution.logo` still gets the letter. **Upgrade notes:** if your
  `vumlo.toml` has `logo = "brand/logo.svg"` and you did not replace that file, change it to `brand/logo.png` or point it
  at your own logo; a logo of your own needs no change.

## [0.2.0] - 2026-10-20

The desktop app, and the two API conventions it needs.

### Upgrade notes

- `config_version` is still **1**.
- The newest core migration is `0031` (`idempotency` schema; Google sign-in for apps; download links of the desktop app). `vumlo-cli db migrate`
  applies it; take a backup first, as always.
- **The addresses of the web app changed** (ADR-0057). A course is `/course?id=<id>` and an item
  `/course/item?id=<id>&item=<id>` (was `/courses/<id>/items/<id>`); `/courses` and `/courses/new` are as they were.
  Old bookmarks show "not found"; nothing is redirected.
- Run `vumlo-cli brand apply` again before you build the web app: `client-config.json` has a new `desktop` part
  (the scheme of the desktop app's links) and the generated directory a 1024-pixel `icon-1024.png`.
- New optional section `[desktop]` with `url_scheme`. Nothing to do unless you build the desktop app.
- Cross-origin frontends (ADR-0041) may now send `Idempotency-Key` and `If-None-Match`, and read `ETag`.

### Added

- **A desktop app** (ADR-0056, ADR-0060, ADR-0061, ADR-0062, ADR-0063; `docs/desktop.md`): Tauri 2 for Linux,
  Windows and macOS, built by each institution from its own `vumlo.toml` (`desktop/`). It reads courses,
  materials, deadlines and grades without a connection, keeps a student's drafts and submits in a send queue that
  the server always has the last word on, stays in the tray, announces deadlines, opens `vumlo-<abbr>://` links,
  keeps the session in the operating system's keychain and signs in with Google through the browser.
  `desktop/release/desktop-release.yml` builds and (with certificates) signs the installers.
- `Idempotency-Key` on `POST` (ADR-0058) and `ETag` / `If-None-Match` on `GET` (ADR-0059), for every client.
- `POST /auth/app/exchange` and an optional `app_challenge` on `POST /auth/google/start` (ADR-0062).
- **Download links of the desktop app** (ADR-0064): an administrator pastes the installers' addresses on the
  Institution page (`PUT /admin/institution/app-downloads`); `GET /institution` lists them. With at least one, the
  home page shows a notice with a button for the visitor's system, there is a public page `/download`, and the account
  menu has an entry. No links, none of it.

### Changed

- **A teacher can publish a course** from the course page: a draft course shows a notice ("Students cannot see it yet,
  even those you have added") with a "Publish course" button, and a Draft badge by the title. The endpoint existed
  since 0.1.0, but no screen used it, so a new course could not be shown to students from the web app.
- The web app builds in two modes: the server build as before, and a static export for the desktop
  (`npm run build:desktop`). Ids are in the query string in both.
- A student's pending draft is sent when they leave the page (it used to be dropped).
- `vumlo-cli brand apply` makes every icon from a 1024-pixel master.

## [0.1.0] - 2026-10-19

The first release: a complete core for one school or university, and the tool to install it.

### Upgrade notes

There is nothing to upgrade from; follow [docs/install.md](docs/install.md).

- `config_version` is **1**. A newer value in `vumlo.toml` is refused.
- The newest core migration is `0028`. `vumlo-cli db migrate` applies the migrations; a database that
  is newer than the binary is refused. Take a backup first (`vumlo-cli db backup`, or
  `deploy/update.sh`, which does it for you).
- The institution mode (`school` or `university`) is chosen before the first start and cannot be
  changed afterwards (ADR-0031).

### Added

- **Installation** (ADR-0054, ADR-0055): `vumlo-cli`, the administrator tool: `init`, `config check`,
  `config show`, `modules list`, `brand apply`, `build web`, `db status`, `db migrate`, `db backup`,
  `admin create`, `seed --demo`, `doctor` and `version`. One configuration schema (`vumlo-config`) is
  shared by the server and the CLI; every key is `public`, `private` or `secret`, and secrets are read
  from the environment only. `doctor` fails when the CLI, the server and the web app are not one release.
- **Deployment templates** in `deploy/`: a Docker Compose file (PostgreSQL, core, worker, web), the
  Dockerfiles, `vumlo.example.toml`, `env.example`, hardened systemd units, a Caddyfile and `update.sh`
  (backup, stop, migrate, start, check). The `./vumlo` script runs the CLI from a local binary or from
  the compose service. Each release is a source archive with a checksum file, attached to its GitHub
  Release; its builds are tested from scratch in CI.
- **Placeholder brand**: the name "Vumlo" and a marked placeholder logo, `brand/logo.svg`, so that
  `init`, `brand apply` and the build work with no assets of your own. Replace the logo; `brand apply`
  generates everything from it again.
- **Branding at build time**: the institution's name, short name, language, time zone, grading scale and
  logo from `vumlo.toml` become the web app's configuration, favicon, icons and manifest
  (`brand apply`). There is no runtime branding endpoint and no theme settings.
- **Optional modules**: a Cargo feature plus `[modules]` in `vumlo.toml`. None is included in this
  release; the mechanism is tested with a test-only module.
- **Identity** (ADR-0003, ADR-0017, ADR-0038, ADR-0046, ADR-0047): password login for web and app
  clients, sessions, login hardening, self-registration limited to `auth.allowed_domains`, email
  verification, password reset and Google sign-in. Administrators create accounts with a temporary
  password that is shown once.
- **Organisation and teaching** (ADR-0005, ADR-0006, ADR-0032, ADR-0033, ADR-0037, ADR-0048):
  terms, groups and subgroups, courses and templates, group links and the live roster, course
  archiving and deletion, course owners.
- **Content** (ADR-0010, ADR-0019, ADR-0020, ADR-0024, ADR-0025, ADR-0026, ADR-0049): the content
  tree of modules with text items and assignments, block documents, self-check questions, publishing
  with dates, unpublished revisions and class dates.
- **Assessment and gradebook** (ADR-0008, ADR-0009, ADR-0011, ADR-0015, ADR-0021, ADR-0023,
  ADR-0028, ADR-0035, ADR-0050, ADR-0051): the `text`, `no_submission`, `quiz` and `form` assignment
  types, submissions with drafts, attempts and deadlines, quiz auto-scoring, draft and published
  grades, and the grade history. Students never see draft grades or answer keys.
- **Mail and events** (ADR-0007, ADR-0045): a transactional outbox and a worker that sends mail.
- **API** (ADR-0012, ADR-0018, ADR-0039, ADR-0041): everything under `/api/v1`, OpenAPI at
  `/api/v1/docs`, Problem Details errors, cursor pagination, opt-in trusted cross-origin frontends.
- **Home and admin read models** (ADR-0052, ADR-0053): course cards, the work list, search, admin
  reports and the audit log, which records every administrator action and every CLI command that
  changes the database.
- **Web app** (ADR-0042, ADR-0043): Next.js, client-rendered, with a Ukrainian and an English interface.

### Not in this release

- File uploads: the `file` assignment type and file and link items are designed (ADR-0023, ADR-0024)
  but not built.
- Any optional module, and an end-user CLI (ADR-0055).
- Published container images: the images are built on your server from the archive.

[Unreleased]: https://github.com/Vumlo/vumlo-core/compare/v0.6.1...HEAD
[0.6.1]: https://github.com/Vumlo/vumlo-core/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/Vumlo/vumlo-core/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/Vumlo/vumlo-core/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/Vumlo/vumlo-core/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/Vumlo/vumlo-core/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Vumlo/vumlo-core/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/Vumlo/vumlo-core/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/Vumlo/vumlo-core/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/Vumlo/vumlo-core/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Vumlo/vumlo-core/releases/tag/v0.1.0
