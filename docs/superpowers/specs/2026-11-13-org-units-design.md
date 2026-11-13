# Org units and unit roles: design

Status: backend built (PR A), web to come (PR B). ADR-0094. Delivery: one spec, two stacked PRs (backend, then web).
Where the build departs from the first design, the text below says what was built (section 13 lists the changes).

## 1. Intent

Today a course is seen by its own staff, its students and admins: nothing in between. An institution has faculties and
departments, and the people who run them (a dean, a head of department, a methodologist) need to see and run the
courses of their part of the institution without being admins and without being added to every course.

Deliver:

1. Units: a tree (`org.units`). A course sits in at most one unit.
2. Unit roles: a person is `manager` or `viewer` of a unit (`org.unit_roles`). A role on a unit covers that unit and
   every unit below it.
3. What each role may do is a set of switches the admin changes on an admin page, at any time, effective at once
   (`org.unit_role_abilities`). Defaults: a manager oversees (sees courses, gradebooks and submissions, creates and
   runs courses, manages rosters and the subtree); a viewer sees.
4. `access` stays the only place that decides: `CourseAccess` carries the actor's unit grant for the course, and each
   rule says which ability lets a grant through.

Goals: contextual roles that follow the tree; the admin decides what the roles mean; no change for anyone who holds
no unit role.

Non-goals: roles other than `manager` and `viewer`; per-unit or per-person ability sets; units for groups (see 9);
moving the institution's settings into units; multi-tenancy.

Anti-goals (a failure even if it works): a unit role that lets someone delete a course or take its ownership; a
student who holds unit powers; a rule that lives outside `access`; a second copy of the tree that can drift.

## 2. Approaches

| | How the subtree is found | Trade-off |
|---|---|---|
| **A (picked)** | `org.units` stores `parent_id`; the view `org.unit_grants` walks the tree with a recursive query on read. | No copy to keep in step, nothing to rebuild; a few ms per course load. A large tree can later swap the view's body for B behind the same name. |
| B | `org.unit_closure` (ancestor, descendant) kept in step on every write. | Plain joins, but a second copy of the tree, locked updates on moves, and a rebuild command. Pays off only with thousands of units. |
| C | Postgres `ltree` path column. | New extension; a move rewrites every path below. |

The user picked A. On ability storage the user picked "an admin page, any time" over install-time config.

## 3. Decisions

| Topic | Decision |
|---|---|
| Roles | `manager`, `viewer`. Fixed. One role per person per unit. |
| Who may hold one | Institution `teacher` or `admin`. Granting to a `student` is `422`; a grant a student somehow holds is ignored by `CourseAccess::for_actor` (as for course roles, ADR-0037). |
| Inheritance | A role on unit U covers U and every unit below it. Several roles covering one unit give the union of their abilities. |
| Abilities | Implicit to any grant: **view** (the course in every status, its roster, its content including drafts). Switches: `view_grades`, `edit_content`, `grade`, `manage_roster`, `manage_courses`, `manage_units`. |
| Defaults | `manager`: `view_grades`, `manage_courses`, `manage_roster`, `manage_units`. `viewer`: `view_grades`. |
| Never through a unit | `DeleteCourse`, `TransferOwnership`, and every student action (`SubmitWork`, `MarkDone`, `ViewOwnGrades`, `ViewOwnSubmission`). |
| Where the vocabulary lives | `access` (`UnitRole`, `UnitAbility`, `UnitGrant`), like `CourseRole`. `org` stores text, constrained by `CHECK`. |
| Changes take effect | At once: nothing caches abilities; every request reads the view. |
| Audit | Granting, changing and revoking a unit role, changing a role's abilities, creating, renaming, moving and deleting a unit: one `events.audit_log` row each (ids and names only). **And every change inside a course made through a unit grant**, like an admin's (user's decision, section 5a). |
| Demotion | When a teacher becomes a student, `learning::people::update_user` deletes their unit roles (an `org` function, one audit row each), as it drops their teacher memberships. `PUT /units/{id}/roles/{user_id}` reads the person's institution role with the user row locked (as the course-role code does), so a grant cannot race a demotion. (Builder check; my call.) |
| Archived courses | Moving a course between units is allowed while it is archived: it changes where the course sits, not the course (as archive and restore ignore the status). `ManageUnitCourses` carries no `CourseAccess`, so the archived check does not apply. (Builder check; my call.) |

## 4. Data (migration `0055_org_units.sql`)

```sql
CREATE TABLE org.units (
    id         uuid PRIMARY KEY,
    parent_id  uuid REFERENCES org.units (id),          -- no cascade: a unit with children is not deleted
    name       text NOT NULL CHECK (name = btrim(name) AND char_length(name) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT units_name_key UNIQUE NULLS NOT DISTINCT (parent_id, name)
);
CREATE INDEX units_parent_id_idx ON org.units (parent_id);

CREATE TABLE org.unit_roles (
    unit_id uuid NOT NULL REFERENCES org.units (id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES auth.users (id),
    role    text NOT NULL CHECK (role IN ('manager', 'viewer')),
    PRIMARY KEY (unit_id, user_id)
);
CREATE INDEX unit_roles_user_id_idx ON org.unit_roles (user_id);

CREATE TABLE org.unit_role_abilities (
    role    text NOT NULL CHECK (role IN ('manager', 'viewer')),
    ability text NOT NULL CHECK (ability IN
            ('view_grades', 'edit_content', 'grade', 'manage_roster', 'manage_courses', 'manage_units')),
    PRIMARY KEY (role, ability)
);
-- seeded with the defaults of section 3

ALTER TABLE teaching.courses ADD COLUMN unit_id uuid REFERENCES org.units (id);   -- no cascade
CREATE INDEX courses_unit_id_idx ON teaching.courses (unit_id);
```

A template course (`kind = 'template'`) may sit in a unit too; a copy made from it does not inherit the unit (the
creator picks one, section 6).

**The declared view** `org.unit_grants (user_id, unit_id, abilities text[])`: one row per person and per unit their
roles cover, `abilities` the union over every covering role. A recursive CTE from each `unit_roles` row down through
`parent_id`, joined with `unit_role_abilities`, grouped by `(user_id, unit_id)`. Added to `DECLARED_VIEWS`;
`teaching` reads only declared views. A second declared view, `org.unit_subtree (root_id, unit_id)` (every unit with
itself and each unit below it), is the base of `org.unit_grants` and lets `teaching` list the courses of a unit for an
admin, who has no grant rows. Both have `CYCLE` detection in their recursive part. sqlx sees view columns as nullable:
`AS "col!"`.

**No cycles.** A move (`parent_id` change) takes `LOCK TABLE org.units IN SHARE ROW EXCLUSIVE MODE`, then refuses
(`422` on `parent_id`) a new parent that is the unit itself or below it. Every write to `parent_id` goes through that
one service function. The recursive view still uses `CYCLE` detection (PG 14+) as a backstop, so a bad row can never
hang a query.

## 5. Access

```rust
pub enum UnitRole { Manager, Viewer }
pub enum UnitAbility { ViewGrades, EditContent, Grade, ManageRoster, ManageCourses, ManageUnits }
/// The union of the abilities of every unit role that covers one unit. Present = view.
/// Carries the `unit_id` of the course's unit, so the audit row can name it.
#[derive(Clone, Copy)] pub struct UnitGrant { pub unit_id: Uuid, /* abilities: a u8 bitset */ }

pub struct CourseAccess { pub role, pub is_owner, pub status, pub unit_grant: Option<UnitGrant> }
```

`for_actor` clears `unit_grant` for an institution student. The rules change only by OR-ing the grant in:

| Action | Unit grant lets it through when |
|---|---|
| `ViewCourse`, `ManageContent`, `ViewContent` | any grant (every status, drafts included) |
| `ViewGradebook`, `ViewSubmissions` | `view_grades` |
| `EditContent` | `edit_content` |
| `GradeAssignment`, `PublishGrades`, `ManageGradebook` | `grade` |
| `ManageRoster`, `ChangeRoster`, `ManageMembership{any role}`, `LinkGroups`, `ChangeGroupLinks` | `manage_roster` (for `ManageMembership`, never on the actor's own row: see below) |
| `EditCourse`, `PublishCourse`, `ArchiveCourse` | `manage_courses` |
| `DeleteCourse`, `TransferOwnership`, student actions | never |

The archived-course check after the permission stays as it is.

`ManageMembership` carries `target_is_actor` (the member put, changed or removed is the actor). A permission that comes
only from the unit grant (not admin, not course owner or staff) is refused when it is set: a manager adds anyone else
at any course role, but never puts themselves on the roster, changes their own role or removes themselves, or they
could make themselves the course's staff past the admin's switches, and unaudited.

`effective_unit_grant(actor, unit_id, held)` says what grant an actor holds on a unit from `held`, the union of their
roles there: an admin every ability on every unit, a student none, anyone else `held`. `org` asks it rather than
deciding who is an admin itself.

New actions, each with a `UnitGrant` (or `None` for "no grant on that unit") inside:

- `ViewOwnUnits`: anyone signed in (as `ViewOwnCourses`); authorizes `GET /units`, which then shows only what the actor's grants cover.
- `ViewUnit(Option<UnitGrant>)`: admin, or any grant.
- `ManageUnitCourses(Option<UnitGrant>)`: admin or `manage_courses`. Create a course in a unit; put a course into a
  unit or take it out (needs it on the old unit and on the new one; "no unit" needs only the old).
- `ManageUnits(Option<UnitGrant>)`: admin or `manage_units`, checked against the **parent** of the unit acted on:
  create a subunit under P (grant on P), rename, move or delete U (grant on U's parent, and on the new parent for a
  move), grant or revoke roles on U (grant on U's parent). A top-level unit (no parent) is admin-only. So nobody
  changes the unit their own role sits on, or their own role.
- `ManageUnitRoleAbilities`: admin only.

Pure tests in `access/src/tests.rs` cover each row of the table, the student case and the "never" row.

### 5a. Audit of implicit access

Today `events::audit_admin(conn, actor, entry)` writes only when `implicit_course_owner(actor)` (the actor is an
admin). The reason is the same for a unit grant: the actor is not on the course, so the course does not show who acted.
The test moves from the actor to the access:

- `access` gets `implicit_access(actor, access) -> Option<Via>` (`Via::Admin | Via::Unit`): the actor acts on the course
  without being its staff. An admin who created the course is its `owner_id` with no membership, and is still audited,
  so the test is "the actor is an admin (`Via::Admin`), or holds a unit grant and is not the course's owner, teacher or
  assistant (`Via::Unit`)": `has_unit_grant && !is_staff`. A grantee who is a student of the course writes through the
  grant and is audited. A later kind of contextual access adds a `Via`, with no change to callers.
- `events::audit_admin` became `events::audit_implicit(conn, actor, access, entry)` (the actor comes before the access),
  writing when `implicit_access` is `Some`. Its callers (teaching, content, assessment, gradebook) pass the `CourseAccess`
  they already authorized with; where one did not hold it, its loader returns it (as `load` does).
- The audit row records how the actor got in: `meta.via = "admin" | "unit"` (and `unit_id` for a unit grant).
- Tests: one API test per crate that a manager's write is audited and a course teacher's is not.

### 5b. What the web app is told

A unit grantee has no course role, so the course response (`GET /courses/{id}`) carries `can`: the course-level
actions this actor may take, computed by `access` from the same `CourseAccess` (`view_grades`, `edit_content`,
`grade`, `manage_roster`, `manage_course`; staff and admins get theirs too). The web app reads `can` instead of
guessing from the role. Required in PR A (builder check; my call).

## 6. Teaching

- `load` and `find_course` / `list_courses_of` read `org.unit_grants` for `(actor, course.unit_id)` beside
  `effective_members`; the locked path reads it in the second statement, after the lock, for the reason the comment on
  `load` gives for the role.
- `access_of` fills `unit_grant`.
- `POST /courses` takes an optional `unit_id`; with one it authorizes `ManageUnitCourses` on that unit as well as
  `CreateCourse`.
- Moving a course is its own operation, `PUT /courses/{id}/unit` `{unit_id: uuid|null}` (`null` takes the course out),
  not a field of `PATCH /courses/{id}`: `PATCH` needs `EditCourse`, which refuses an archived course, and a move is
  allowed on an archived one (section 3). Authorized as in section 5 (`ManageUnitCourses`).
- Copying a course or creating from a template (`POST /templates/{id}/courses`) takes the `unit_id` of the request, not the source's.
- `GET /me/courses` is unchanged (only courses one belongs to). Unit courses are listed per unit (section 7).
- The `Course` response carries `unit_id`.

## 7. API (new, in `org` unless noted)

| Method and path | Action | Notes |
|---|---|---|
| `GET /units` | `ViewOwnUnits`: any signed-in user | Flat list `{id, parent_id, name, abilities}` of every unit the actor sees: admin all (abilities: all), others the units their grants cover. Empty for most people. It is the items envelope with `next_cursor: null`: not paginated, the whole tree comes in one response. |
| `POST /units` `{name, parent_id?}` | `ManageUnits` on the parent | `201` |
| `PATCH /units/{id}` `{name?, parent_id?}` | `ManageUnits` on old (and new) parent | cycle → `422`. A request with both `name` and `parent_id` is one statement. |
| `PUT /courses/{id}/unit` `{unit_id}` (in `teaching`) | `ManageUnitCourses` on the old and the new unit | Puts the course in a unit or, with `null`, takes it out. Its own operation, not a field of `PATCH /courses/{id}`; allowed on an archived course. |
| `DELETE /units/{id}` | `ManageUnits` on the parent | `409` if it has subunits or courses |
| `GET /units/{id}/roles` | `ViewUnit` | `{user_id, name, email, role}` of roles **on** this unit |
| `PUT /units/{id}/roles/{user_id}` `{role}` | `ManageUnits` on the parent | student → `422`; idempotent |
| `DELETE /units/{id}/roles/{user_id}` | `ManageUnits` on the parent | |
| `GET /units/{id}/candidates?q=` | `ManageUnits` on the parent | Teachers and admins by name or email, as `/courses/{id}/candidates` does for a roster. |
| `GET /units/{id}/courses?cursor=&limit=` (in `teaching`) | `ViewUnit` | Courses in U and below, every status, cursor pagination (ADR-0018). |
| `GET /admin/unit-roles` | `ManageUnitRoleAbilities` | `{manager: [..], viewer: [..]}` |
| `PUT /admin/unit-roles/{role}` `{abilities: [..]}` | `ManageUnitRoleAbilities` | replaces the set in one transaction; unknown ability → `422` |

Every new operation is listed in `server/tests/authorization.rs`. `web/openapi.json` and `web/src/lib/api/schema.d.ts`
are regenerated. Beyond this table, `GET /courses/{id}` carries `can` (5b) and `unit_id`, and `POST /courses` and
`POST /templates/{id}/courses` take an optional `unit_id`.

## 8. Web

- **Admin → Units** (`web/src/app/(app)/admin/units`): the tree (create, rename, move, delete), the roles on a
  selected unit (add by search, change, remove), and a "Unit roles" panel with one switch per ability for `manager`
  and `viewer`, each with a one-line explanation. `view` is shown as always on.
- **Units** for non-admins (`/units`, shown in the navigation only when `GET /units` is non-empty): the same tree
  limited to what the person sees, each unit's courses, and the tree and role controls only where `manage_units`
  allows (on children of the units they hold it on).
- Course create and course settings: a unit picker when the person may place courses (admin, or `manage_courses`
  somewhere).
- Course pages reached through a unit work as for staff, with controls shown by the course response's `can` (5b).
- i18n strings in every catalogue the web app ships.

## 9. Out of scope, said out loud

- **Groups in units.** The roadmap's first sketch put `unit_id` on groups. Nothing would read it until a manager can
  manage the unit's groups, which needs a people picker for members and its own rules. Groups stay admin-managed;
  linking any group to a unit course is covered by `manage_roster`.
- Unit-level reports and dashboards (the admin reports stay institution-wide).
- Installer questions about units (the "more setup choices" roadmap item).
- Deleting a unit that still has courses (move them first).
- **Copying a course or making a template needs only `ManageContent`**, which any unit grant holds, so a view-only
  grantee can copy what they can already read (drafts included) into a course of their own. Deliberately unchanged in
  PR A: the copy belongs to the copier and reveals nothing they could not read. Flagged for review.

## 10. Delivery

- **PR A, backend:** migration 0055, `access` types and rules with tests, `org` units, roles, abilities and routes,
  `teaching` loading, create/patch/list, audit rows, authorization tests, `sqlx prepare --all-features`, OpenAPI.
  ADR-0094 line in `docs/decisions/in-progress.md`, roadmap row updated.
- **PR B, web:** the admin page, the units page, the unit picker, i18n, e2e test of a manager seeing a unit course and
  an admin switching `view_grades` off for `viewer`.

## 11. Built to be extended

The user's standing requirement: a later change must not break the system. Each likely extension has one place:

| Extension | Where it goes | What else changes |
|---|---|---|
| A new ability | a `UnitAbility` variant in `access` (the compiler asks for its bit; a test fails until `UnitAbility::ALL` holds it) and in `api-types`, its row in the rule table (section 5), a migration widening the `CHECK` | the view and the admin page's switches follow (they list what `GET /admin/unit-roles` offers); `can` gets a field only if the web app must show the new action; OpenAPI and web types are regenerated |
| A new unit role | a `UnitRole` variant in `access` and `api-types`, a migration widening the `CHECK` and seeding its abilities | `UnitRoleAbilities` (the `GET /admin/unit-roles` response, one field per role) gets a field, and the admin page shows it |
| Groups (or anything else) in units | a `unit_id` column on the owner's table; its rules read `org.unit_grants` | `org` and `access` unchanged |
| Another kind of contextual access | a field on `CourseAccess` and its rows in `authorize` | audit follows `implicit_access` (a new `Via`), no caller changes |
| A large tree | the body of `org.unit_grants` (section 2, B) | no reader changes |

The invariants that keep it so: rules only in `access`; `org` owns units, roles and abilities and is read only through
`org.unit_grants`; nobody caches a grant; every write path to `parent_id` goes through one function.

## 12. Left to the builder

The exact Rust representation of `UnitGrant`; how `org` and `teaching` split the queries (within the declared-view
rule); response shapes beyond the fields named; page layout within the existing admin style.

## 13. Changes made while building (PR A)

1. Moving a course is `PUT /courses/{id}/unit`, not a field of `PATCH /courses/{id}` (sections 6 and 7).
2. A second declared view, `org.unit_subtree (root_id, unit_id)`, under `org.unit_grants` (section 4).
3. Admins stay audited on courses they own: implicit access is "an admin, or only a unit grant", `via` is `admin` or `unit` (5a).
4. `UnitGrant` carries the `unit_id` of the course's unit (section 5).
5. `GET /units` is authorized by `Action::ViewOwnUnits` and is not paginated (sections 5 and 7).
6. `like_pattern` (the `ILIKE` escaper) moved from `teaching::service::info` to `common`, so `org` can search candidates with it.
7. `unit_roles.user_id` references `auth.users` (the schema of the users table), and `events::audit_implicit` takes `(conn, actor, access, entry)`.
8. From the whole-branch review: implicit access is `has_unit_grant && !is_staff` (5a); `ManageMembership.target_is_actor`
   and `effective_unit_grant` (section 5); `PUT /courses/{id}/unit` requires `unit_id` (`null` to clear, `{}` is `422`);
   an unknown or unseen `parent_id` in a unit request body is `422` on `parent_id`; a plugin launch token says `staff`
   only to whoever may read the submissions (`ViewSubmissions`), `student` to a course student, and is refused to anyone
   else; the counts to grade on the item facts need `ViewSubmissions`.
