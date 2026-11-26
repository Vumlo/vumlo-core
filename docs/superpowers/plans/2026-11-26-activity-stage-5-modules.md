# Stage 5 plan: modules add activity types, an unregistered stored type is `Missing`

Spec: `docs/superpowers/specs/2026-11-26-activity-generalization-design.md` (ADR-0098), stage 5 of 5, decisions D4 and
section 9. Branch `refactor/activity-modules`, stacked on `refactor/activity-resources`.

## What changes

1. **`activity`**: `Missing`, the stand-in for a stored id nobody registered. `Kind` and `GradedKind` decode a stored id
   through `stored` / `stored_graded`, which fall back to it; `get` and `get_graded` stay strict, so a creator still
   refuses an unknown id. `ActivityType::available` (default `true`, `false` for the stand-in) and `Kind::is_missing`.
   The stand-in has completion `Graded`, no submissions, a body that cannot be written and a config that cannot be
   validated; an assignment of it keeps its points. `activity::DEFAULT_RESOURCE` replaces the one `lecture` literal in
   `content`.
2. **`module-api`**: `Module::activity_types()` (default none). `module-api` now lists `activity` (the layer test allows
   it). A module's types are static values that need no state, so they are collected before `AppState` exists.
3. **`server` and `vumlo-cli`**: `modules::activity_types(settings)` / `runtime::activity_types(settings)` build the
   registry from the built-ins and the types of the compiled-in modules that are **enabled**; `install_registries` takes
   the settings. A module that is off adds nothing, so its items read as unavailable until it is enabled again.
4. **`content` and API**: `Item.unavailable` and `ItemSummary.unavailable` (sent only when true); the body and the
   assignment's config of such an item are empty in every response; search matches its title only. Writes are refused
   by the stand-in (`validate_body`, `validate_config`), a student cannot mark it done (policy `Graded`).
5. **`doctor`**: the check `activity types` warns with the ids and counts of stored items whose type this server does not
   register (`content::stored_types`).
6. **`dummy` module**: the type `dummy_note`, the smallest type a module can add.
7. **Enforcement**: `architecture.rs` fails when a built-in type id (`types::<name>` or the quoted id) appears outside
   `activity`, `modules`, `vumlo-cli` and tests.
8. **Web**: `UnavailablePage` for an unavailable item (viewer and editor), a badge in the student's list, two strings.

## Verification

- `activity`: the stand-in tests (`registry/tests.rs`).
- `server` with `--features dummy`: `module_activity_types` (a module type is created, published, marked done, an unknown
  type is refused) and `module_activity_types_off` (the same world with the module off: the tree loads, the body and
  config are empty, search does not quote it, nothing can be written). Two binaries, because the registry is
  process-wide.
- `architecture`, `module_layers`, the whole server suite, clippy in the three feature sets, `sqlx prepare --check`,
  web `typecheck` and `build`.

## Not in this stage

The module's own web page for its type (ADR-0071), and a way to list unavailable items for an administrator beyond
`doctor`.
