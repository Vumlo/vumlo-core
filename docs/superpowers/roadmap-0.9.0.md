# Roadmap 0.9.0

Status on 2026-11-13. Source: the architecture sketch of the finished core (`~/Downloads/lms-architecture-overview.md`), compared with the repository. The structural blockers to adding modules were the single big `learning` crate, with calls and reads that went in a circle, and the closed assignment-type enum `content::model::BuiltinType`.

Every item below is done the same way: an ADR line (`docs/decisions/`), a spec (`docs/superpowers/specs/`), a plan (`docs/superpowers/plans/`, written with the `writing-plans` skill), then stacked PRs, with a stop for review after each PR.

## Order

| # | Item | State |
|---|---|---|
| 1 | Split `learning` into crates (ADR-0093) | Built. PR A #61, PR B #62, files-usage #63 merged; PR C #64 open |
| 2 | Org units and contextual roles (ADR-0094) | PR A (backend) built on `feat/org-units`, awaiting review; PR B (web) not started. Spec: `docs/superpowers/specs/2026-11-13-org-units-design.md` |
| 3 | Activity type contract, replacing `BuiltinType` | Not started |
| 4 | Progress: completion policies | Not started |
| 5 | New modules: final grades, question bank, notifications, calendar, forums | Not started |
| 6 | Interop: Moodle XML/GIFT, QTI, LTI 1.3, SCORM, H5P | Not started |
| 7 | More setup choices for the admin when a Vumlo is created (installer and admin settings) | Not started; brainstorm after item 2 |

### 1. Split `learning` into crates (ADR-0093)

Five crates in `backend/crates/learning/`: `teaching < content < assessment < gradebook < learning` (the last, in `flows/`, holds only the flows, the scheduler and the audit report). Reads across crates go only through views the owner declared. A call that would go up the order is a flow in `learning` or a synchronous event subscriber. `content.facts` is a projection updated in the same transaction and rebuildable with `vumlo-cli facts rebuild`.

- PR A (#61): `events::install` and the synchronous subscribers, `content.facts`, the quiz auto-grade as a subscriber.
- PR B (#62): nothing goes up the order (flows, `begin_*` / `finish_*` pairs); migration `0054` with eleven read views.
- Files-usage (#63): `learning::files::check` locks the file rows, and the writes that name files run `begin_*`, `check`, `finish_*` in one transaction.
- PR C (#64): the physical move into five crates, with a state, error enum and router each.

### 2. Org units and contextual roles (ADR-0094)

Design agreed earlier:

- `org.units`: a tree.
- `org.unit_roles`: a person is `manager` or `viewer` of a unit.
- `unit_id` on courses and groups.
- A view `org.unit_grants`, read by the layers above (it is a declared view).
- `CourseAccess.unit_grant`: a unit role gives access to the courses of that unit and of the units below it.

Spec and plan written; PR A (backend) is built: units, `manager`/`viewer` roles, abilities the admin switches, `org.unit_subtree` and `org.unit_grants`, `CourseAccess.unit_grant`, `audit_implicit`, `PUT /courses/{id}/unit`, `GET /units/{id}/courses`, `can` on `GET /courses/{id}`. `unit_id` on groups was left out (spec section 9). Next: PR B (the web pages).

### 3. Activity type contract

Replaces the closed enum `content::model::BuiltinType` with a contract an activity type implements: its capabilities and its completion policy.

### 4. Progress

Completion policies, then `activity.completed` and `course.completed`. A policy decides; completion never comes straight from `submission.submitted`.

### 5. New modules

Final grades, question bank, in-app notifications, calendar, forums. Each is a crate or an optional module with its own schema, built on the contract of item 3.

### 6. Interop

Moodle XML and GIFT, QTI (into the question bank), LTI 1.3, SCORM, H5P (as activity types).

## Rules that hold for all of it

- Cross-crate reads only through declared views. `assessment` and `gradebook` own their data; `content` owns the access rules and its `facts` projection.
- The synchronous projection update happens in the same transaction (subscribers); the outbox stays the reliable asynchronous delivery. Handlers are idempotent and projections can be rebuilt from the sources.
- Everything is still in development: break and rebuild freely, clean structure and reliability first. No side endpoints, shims or cramped patches; re-check the structure before opening a PR.
- Each API operation is listed in `server/tests/authorization.rs`.
- CI runs the full suite; locally only targeted tests are run (`CARGO_BUILD_JOBS=2`, one at a time). Always check `--workspace --all-targets --all-features`, and build each new crate alone with `cargo test -p <crate> --no-run`.

## Not on the roadmap

- Cleanup jobs from ADR-0007 in the scheduler: expired sessions, retention of processed outbox rows and of the audit log.
- Deferred minors of #62 and #63: `copy_course_content` copies file references without locking the files; the guard that only flows complete a write naming files exempts whole folders and does not scan `vumlo-cli`; a Postgres deadlock abort (40P01) has no dedicated error mapping.
