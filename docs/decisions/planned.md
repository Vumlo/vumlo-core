# Decisions: planned

Not built. The first group has an ADR (0055, deferred on purpose). The rest comes from the "Not in v1", "planned"
and "Open questions" parts of `ARCHITECTURE.md` and has no ADR yet; each one needs an ADR before it is built.

## Deferred on purpose
- ADR-0055 — Not built, not reserved in code: widgets, an end-user CLI (the name `vumlo-cli` belongs to the admin tool),
  native mobile clients, the module `vpl`, release binaries, down migrations.
  Third-party plugins and authenticated module routes were added later by 0066; the module `ai_agent` by 0070 (in progress).

## Accepted, not built
- ADR-0098 — Activities are one concept in one crate. Every item of a course module is an activity of a registered type: a lecture and a shared file become types (`lecture`, `shared_file`) beside the five assignment types, `content.items` gets `type` and its `kind` only says graded or not (`resource` or `assignment`), and the five built-in types move from `content` into the crate `activity`, whose contract becomes tiers (`ActivityType`, `Graded`, `Submissions`, `AutoGrading`). The features a type owns (the quiz result, the form responses and their "taken" overlay, the plugin reads) are hooks of the contract, so no crate outside `activity` compares a type id (a module, the CLI and tests may name a built-in); the plugin names that say `quiz` become type-neutral (`attempts:read`, `attempt.autograded`, plugin `api = "2"`). A module registers a type through `Registrar`, and a stored type nobody registered reads as `Kind::Missing`. Five stages: move, tiers, hooks, activities with the migration, modules. Spec: `docs/superpowers/specs/2026-11-26-activity-generalization-design.md`.

## No ADR yet
- Final and term grades: where they live, periods, computation, rounding (direction in 0031: teacher enters, system suggests).
- What `institution.mode` should change beyond the default grading scale: terminology, dashboards.
- Guardians: an institution role linked to specific students.
- Webhook subscriptions with HMAC and retries (`ARCHITECTURE.md` lists them as planned).
- CLI login (device flow) and personal access tokens: the sessions exist in the data, no endpoints.
- Server command `import-users`; the config key `editor.html`.
- Group ownership, so a teacher can see group members without linking the group.
- Multi-tenancy, configurable roles, LTI 1.3, WASM plugins, realtime updates (listed as not in v1).
