# 0044. The institution profile lives in the config; `chrono-tz`

Status: Accepted. The accent colour (the key `accent` and its five values) is superseded by ADR-0054: there is no accent.
Supplements: ADR-0002, ADR-0022, ADR-0031
Amends: ADR-0014 (adds `chrono-tz`)

## Context
The web app shows the institution's name before anyone signs in, picks the interface language from
the institution default (ADR-0043), writes times in the institution's time zone and tints the site
with an accent. ADR-0002 keeps institution settings in the config file and says they move to a table
"only when admins need to change them from the UI (new ADR)". The project owner decided that nobody
edits them from the UI: the config file stays the one source, and the web app loses its edit form.
The server therefore needs to publish the settings it already holds and to validate the new ones.

A time zone is only valid if it is in the IANA database. `chrono` has no zone database, so checking a
value like `Europe/Kyiv` at startup needs one.

## Options
- Where the profile lives: (a) a one-row table that an admin edits (ADR-0002's escape hatch): a
  migration, a seed, a second place to look and a write API nobody asked for; (b) **the config file,
  parsed once into the typed `InstitutionSettings`**.
- Validating `time_zone`: (a) **`chrono-tz`** with its database compiled in: works in any image and
  later formatting of deadlines and class dates can reuse it; (b) the system `tzdata`: the Docker
  image (`debian:stable-slim`) does not ship it and every dev machine differs; (c) a shape check:
  `Europe/Kyev` would pass.
- Who may read the admin view: (a) a new role check in the handler: forbidden by AGENTS.md section 5;
  (b) **a policy action**.

## Decision
1. New optional keys in `[institution]`, all with defaults, validated at startup (a bad value stops
   the server with a message that names the key and the allowed values, like `institution.mode`):
   - `short_name`: 1 to 50 characters after trimming, default `name`;
   - `language`: `en` or `uk`, default `en` (the catalogues of ADR-0043);
   - `time_zone`: an IANA name known to `chrono-tz`, default `UTC`;
   - `accent`: `indigo`, `teal`, `crimson`, `forest` or `plum`, default `indigo`. These are the values
     the web app maps to its tested palettes.
2. `GET /api/v1/institution` needs no session and answers `{name, short_name, language, time_zone,
   accent, grading_scale}`: what the sign-in page needs. Nothing in it is secret.
3. `GET /api/v1/admin/institution` is the action `ViewInstitutionSettings` (admin only; others get
   403) and answers the same members plus `mode` and `allowed_domains`. It is read-only. There is no
   `PATCH`, no table and no migration: changing a setting means editing the config and restarting.
4. Both are read from the `Settings` the server parsed at startup. No request reads the file.
5. `chrono-tz` 0.10 with `default-features = false` is added to ADR-0014's list: "Data types: IANA
   time zone names and the zone database, for `institution.time_zone`". It is used by `settings` for
   validation and later by whoever formats times for a person.

## Consequences
- A change of name, language, time zone or accent needs a restart. That is the price of keeping one
  source of truth.
- `short_name` defaults to `name`, so an existing `learnly.toml` keeps working unchanged.
- The compiled-in zone database adds about a megabyte to the binary and goes stale until the crate is
  updated; the zones in use change rarely.
- The accent list is a contract with the web app: a new accent is added in both places.
- Logo upload is not decided here; it needs the `files` module.
