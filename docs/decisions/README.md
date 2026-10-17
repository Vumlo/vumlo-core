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
| 0019 | [Publishing a course is a root action](0019-course-publishing.md) |
| 0020 | [Content tree: course_id on every row, one level of sub-modules, dense positions, opaque lesson body](0020-content-tree.md) |
| 0021 | [Assignments and scores: numeric points, max_points column, text type first](0021-assignments-and-scores.md) |
| 0022 | [The grading scale is an institution setting and limits points](0022-grading-scale-setting.md) |
| 0023 | [Default assignment types: text, no_submission, file, form, quiz](0023-default-assignment-types.md) |
| 0024 | [Item kinds: text, link, file, lesson, assignment](0024-item-kinds.md) |
| 0025 | [Block documents (the canvas): format, features, profiles, sandboxed HTML](0025-block-documents.md) |
| 0026 | [Lessons and self-check questions](0026-lessons-and-questions.md) |
| 0027 | [Document types live in api-types; video blocks store a provider and an id; an option is one block](0027-document-types-and-video-blocks.md) |
| 0028 | [Gradebook: grades, publishing, read models](0028-gradebook-grades-publishing-read-models.md) |
| 0029 | [A user has a name and a surname](0029-user-name-and-surname.md) |
| 0030 | [The assignment-row lock covers deletes; the gradebook may ask teaching for course access](0030-gradebook-lock-covers-deletes-and-course-access.md) |
| 0031 | [The institution mode is required and fixed](0031-institution-mode-required-and-fixed.md) |
| 0032 | [Terms, groups and subgroups](0032-terms-groups-and-subgroups.md) |
| 0033 | [Group links, the effective roster and course exclusions](0033-group-links-effective-roster-and-exclusions.md) |
| 0034 | [Admin writes outside a course are audited](0034-admin-writes-outside-a-course-are-audited.md) |
| 0035 | [Submissions: drafts, attempts and deadlines](0035-submissions-drafts-attempts-and-deadlines.md) |
| 0036 | [Each test gets its own database and its own pool](0036-test-database-harness.md) |
| 0037 | [Who is on a course: active students, former students, role changes, owner transfer](0037-who-is-on-a-course.md) |
| 0038 | [Login hardening: sessions against a verified credential, counted attempts, bounded hashing](0038-login-hardening.md) |
| 0039 | [Input the database cannot store, races, a busy database, unreadable bodies](0039-api-input-and-errors.md) |
| 0040 | [Who may link a group to a course](0040-who-may-link-a-group.md) |
| 0041 | [Trusted frontends on another origin](0041-trusted-cross-origin-frontends.md) |
