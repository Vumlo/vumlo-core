# Decisions: planned

Not built. The first group has an ADR (0055, deferred on purpose). The rest comes from the "Not in v1", "planned"
and "Open questions" parts of `ARCHITECTURE.md` and has no ADR yet; each one needs an ADR before it is built.

## Deferred on purpose
- ADR-0055 — Not built, not reserved in code: widgets, an end-user CLI (the name `vumlo-cli` belongs to the admin tool),
  native mobile clients, the module `vpl`, release binaries, down migrations.
  Third-party plugins and authenticated module routes were added later by 0066; the module `ai_agent` by 0070 (in progress).

## No ADR yet
- Final and term grades: where they live, periods, computation, rounding (direction in 0031: teacher enters, system suggests).
- What `institution.mode` should change beyond the default grading scale: terminology, dashboards.
- Guardians: an institution role linked to specific students.
- Webhook subscriptions with HMAC and retries (`ARCHITECTURE.md` lists them as planned).
- CLI login (device flow) and personal access tokens: the sessions exist in the data, no endpoints.
- Server command `import-users`; the config key `editor.html`.
- Group ownership, so a teacher can see group members without linking the group.
- Multi-tenancy, configurable roles, LTI 1.3, WASM plugins, realtime updates (listed as not in v1).
