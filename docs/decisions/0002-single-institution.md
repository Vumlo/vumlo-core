# 0002. One installation = one institution

Status: Accepted

## Context
The system targets schools and universities that host it themselves. Multi-tenancy would add a `tenant_id` to almost every table and every query.

## Decision
- One installation serves exactly one institution. There is no tenant id.
- Institution settings (mode `school | university`, allowed email domains, terminology, default grading scale) live in the config file.
- The mode changes terminology, defaults and dashboards, never the schema.

## Consequences
- Simpler schema and queries.
- A hosted multi-institution offering would mean one deployment per institution, or a large migration later.
- Settings move to a table only when admins need to change them from the UI (new ADR).
