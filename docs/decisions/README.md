# Architecture Decision Records

One file per decision. Write a new ADR when:

- adding a dependency (crate, service, container);
- adding or changing a table that other modules read;
- changing an API convention, a permission rule or an invariant.

Never edit an accepted ADR to reverse it. Write a new one with `Supersedes: ADR-NNNN` and mark the old one `Superseded by ADR-MMMM`.

## Template

```markdown
# NNNN. Title

Status: Proposed | Accepted | Superseded by ADR-MMMM

## Context
What problem, what constraints.

## Options
1. ... — pros / cons
2. ... — pros / cons

## Decision
What we do.

## Consequences
What gets easier, what gets harder, what we must remember.
```

## Index

| #    | Decision |
|------|----------|
| 0001 | [Modular monolith: Rust + PostgreSQL, nothing else](0001-modular-monolith.md) |
| 0002 | [One installation = one institution](0002-single-institution.md) |
| 0003 | [Identity, login and sessions](0003-identity-and-sessions.md) |
| 0004 | [Two-level roles and a central policy](0004-roles-and-policy.md) |
| 0005 | [Courses and templates in one table, copy on create](0005-courses-and-templates.md) |
| 0006 | [Groups (cohorts), live links, course subgroups](0006-groups-and-membership.md) |
| 0007 | [Domain events via transactional outbox](0007-events-outbox.md) |
| 0008 | [Extensible assignment types](0008-assignment-types.md) |
| 0009 | [Grading: draft/published slots, gradebook = assignments](0009-grading-and-gradebook.md) |
| 0010 | [Content publishing](0010-content-publishing.md) |
| 0011 | [Submissions: drafts, deadlines, attempts](0011-submissions.md) |
| 0012 | [One API for web, mobile and CLI](0012-api-for-multiple-clients.md) |
| 0013 | [Separate frontend](0013-separate-frontend.md) |
| 0014 | [Core libraries](0014-core-libraries.md) |
| 0015 | [Gradebook as a separate module](0015-gradebook-module.md) |
| 0016 | [Module layout and transaction handling](0016-module-layout-and-transactions.md) |
| 0017 | [Accounts, passwords and session lifetimes](0017-accounts-passwords-sessions.md) |
| 0018 | [API validation errors and cursor pagination](0018-api-errors-and-pagination.md) |
