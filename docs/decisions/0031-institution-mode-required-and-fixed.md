# 0031. The institution mode is required and fixed

Status: Accepted
Amends: ADR-0002, ADR-0022

## Context
ADR-0002 puts the institution mode (`school | university`) in the config file. ADR-0022 lets `institution.grading_scale` default to `hundred` until the mode exists. That default is wrong for a school: an admin who forgets the key silently gets the university rule (a cap of 100 points per course) instead of the school rule (at most 12 per assignment).

The grading model, and later the final-grade model, depend on the mode. Changing it on a live installation would make the stored data mean something else. So the mode must be an explicit choice made when the installation is set up, and it must not change afterwards.

ADR-0002 and ADR-0022 are accepted records and are not edited; this ADR amends them, as ADR-0030 did for ADR-0028.

## Options
Is the mode required?
- (a) A default, e.g. `university`: a forgotten key still gives the wrong rule without a word.
- (b) **Required, no default.**

How is "it never changes" enforced?
- (a) By the config alone: an edited config silently switches the model of an installation that already holds data.
- (b) **The first start records the mode in the database; every later start compares the config with it.**
- (c) A DB-only setting written by a separate `init` command: one more setup step, and the config file no longer describes the installation.

Where does the record live?
- (a) **`org.installation`**: `org` owns "institution settings" (ARCHITECTURE.md) and its schema exists but is empty.
- (b) `identity` or `events`: the record is not their concern.
- (c) A migration that writes the value: a migration cannot read the config.

## Decision
1. `institution.mode = school | university` is required in the config. There is no default. A missing or unknown value stops the server at startup with a message that names the key `institution.mode` and the allowed values.
2. `institution.grading_scale` stays optional. When it is absent it follows the mode: school gives `twelve`, university gives `hundred`. An explicit value still wins (ADR-0022 allows it). Existing assignments are not revalidated (ADR-0022 is unchanged here).
3. The mode is fixed for the life of the installation. The first start records it in the database. A later start whose config says a different mode refuses to start and names both values. There is no supported way to change it in v1. The record is a one-row table in the `org` schema, written with insert-if-absent and then read, so two instances starting at the same time with the same mode both succeed, and with different modes exactly one wins. The check runs at startup in the composition root, before the server accepts requests, and in the server's own CLI subcommands that use the database (`migrate`, `create-admin`, `create-user`), so that a fresh database gets its mode from whichever runs first.
4. What the mode controls now: only the default grading scale. What it will control later, not decided here: terminology, dashboards and the final-grade model. The direction, also not decided: a final grade (a semester or year grade at a school, the course grade at a university) is entered by the teacher, with a value the system computes and suggests, from the marks at a school and from the sum of points at a university. A later ADR on final grades decides periods, computation and rounding.
5. The ADR-0002 sentence "the mode never changes the schema" is narrowed: the mode does not change tables. It selects defaults, limits and, later, the computation of final grades.

## Consequences
- A school can no longer end up with the university rule by leaving a key out; it fails at startup instead.
- Every existing `learnly.toml` and every `LEARNLY__` environment must add `institution.mode` before the next start. An existing database has no record yet, so its next start records whatever the config says.
- Changing the mode of a running installation is not supported. It would need a new ADR and a data migration.
- `org` gets its first table and a minimal module with one public function.
- Settings still depend on no module: the mode type lives in `settings`, and `org` takes it as a parameter.
