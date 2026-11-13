# Org units, PR A (backend) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Units (a tree), `manager`/`viewer` unit roles whose abilities the admin switches, enforced in `access`, read by `teaching`, audited like an admin's access: the whole backend of ADR-0094.

**Architecture:** `org` owns `org.units`, `org.unit_roles`, `org.unit_role_abilities` and two declared views (`org.unit_subtree`, `org.unit_grants`, recursive, computed on read). `access` owns the vocabulary (`UnitRole`, `UnitAbility`, `UnitGrant`) and every rule; `CourseAccess` gains `unit_grant`. `teaching` reads `org.unit_grants` beside `effective_members` when it loads a course. Course changes made through implicit access are audited by `events::audit_implicit`, keyed on the access, not the actor.

**Tech Stack:** Rust (axum, sqlx, utoipa, garde), Postgres 18. No new crates.

**Spec:** `docs/superpowers/specs/2026-11-13-org-units-design.md` (read it first). PR B (web) gets its own plan after this PR is reviewed.

## Global Constraints

- Repository rules in `CLAUDE.md` hold everywhere: `routes`/`service`/`repo` private, SQL only in `repo.rs`, service functions take `&mut PgConnection` and never `begin`, routes open the one write transaction, deny by default, no role checks outside `access`, tests in `tests.rs` files, DB tests via `db_test`, never `#[sqlx::test]`.
- Migration `backend/migrations/0055_org_units.sql`; append-only; `text` + `CHECK`, never Postgres `ENUM`; UUID v7 ids.
- Roles: `manager`, `viewer`. Abilities (exact strings): `view_grades`, `edit_content`, `grade`, `manage_roster`, `manage_courses`, `manage_units`.
- Defaults seeded: `manager` = `view_grades`, `manage_courses`, `manage_roster`, `manage_units`; `viewer` = `view_grades`.
- A unit role never allows `DeleteCourse`, `TransferOwnership` or a student action.
- Unit names reuse `org::OrgName` (trimmed, 1–100 chars), unique among siblings.
- Lists use `?cursor=&limit=` → `{items, next_cursor}`; validation `422` with `errors: [{field, message}]`.
- Local runs: `CARGO_BUILD_JOBS=2`, targeted tests one at a time; `cargo sqlx prepare --workspace -- --all-targets --all-features` after query changes (a broken build wipes `.sqlx`: restore with `git checkout backend/.sqlx`). Dev DB: `docker compose up -d postgres`, migrations with `sqlx migrate run --source migrations` from `backend/` and `DATABASE_URL=postgres://vumlo:vumlo@localhost:5432/vumlo`. Change files with Edit/Write, not sed/python. Revert `web/pnpm-lock.yaml` churn. Never commit `graphify-out/` or `docs/superpowers/roadmap-0.9.0.md` unless asked.

## Adjustments to the spec (decided while planning)

1. **Moving a course is its own endpoint**, `PUT /courses/{id}/unit` `{unit_id: uuid|null}`, not a field of `PATCH /courses/{id}`. `PATCH` needs `EditCourse`, which refuses an archived course; the spec allows moving an archived one. A separate operation keeps one action per write.
2. **A second declared view `org.unit_subtree (root_id, unit_id)`** (every unit with itself and each unit below it). `org.unit_grants` is built on it, and `teaching` uses it to list a unit's courses for an admin, who has no grant rows.
3. **Admins stay audited on courses they own.** An admin who creates a course is its `owner_id` (with no membership). So implicit access is "the actor is an admin, or they reached the course only through a unit grant". `via` is `admin` or `unit`.
4. `UnitGrant` carries the `unit_id` of the course's unit, so the audit row can name it.
5. `GET /units` is authorized by a new `Action::ViewOwnUnits` (anyone signed in), as `ViewOwnCourses` is.
6. `like_pattern` (the `ILIKE` escaper) moves from `teaching::service::info` to `common`, so `org` can search candidates with it.

## Review Focus

1. **Revoking a role or switching an ability off takes effect on the very next request.** A manager whose `edit_content` was just switched off gets `403`/`404`, not a cached yes. Test in Task 7.
2. **A grant on an ancestor reaches a course two levels down; a grant on a sibling does not.** Tests in Task 1 (the view) and Task 7 (API).
3. **A manager cannot widen their own power.** They cannot change their own unit's roles, rename or move their own unit, or move a unit under a unit they do not manage. Tests in Task 5.
4. **A manager acting on a course leaves an audit row; the course's own teacher does not.** Test in Task 3 (`audit_implicit`) and Task 7.
5. **Deleting a unit that still has courses or subunits is `409`, never a cascade.** Moving a unit under its own descendant is `422`. Tests in Task 4.

---

### Task 1: Migration, views and the architecture rule

**Files:**
- Create: `backend/migrations/0055_org_units.sql`
- Modify: `backend/crates/server/tests/architecture.rs` (`DECLARED_VIEWS`; the "views in migrations read another schema only where an ADR declared it" list, if `org.unit_grants` needs none: it reads only `org`)
- Test: `backend/crates/server/tests/read_views.rs`

**Interfaces:**
- Produces: tables `org.units (id, parent_id, name, created_at)`, `org.unit_roles (unit_id, user_id, role)`, `org.unit_role_abilities (role, ability)`, column `teaching.courses.unit_id uuid NULL REFERENCES org.units(id)` (no cascade), views `org.unit_subtree (root_id, unit_id)` and `org.unit_grants (user_id, unit_id, abilities text[])`. Tables and indexes exactly as spec section 4, seeded with the defaults above.

- [ ] **Step 1: Write the failing tests** in `read_views.rs`, seeding rows with SQL (units A > B > C and a sibling S under A' at top level; Pat is `viewer` of A and `manager` of B):
  - `unit_subtree_holds_each_unit_and_everything_below`: rows for root A are exactly {A, B, C}; for C exactly {C}.
  - `unit_grants_cover_the_subtree_and_unite_the_roles`: Pat on A = `{view_grades}`; on B and C = `{view_grades, manage_courses, manage_roster, manage_units}` (sorted for comparison); no row for S.
  - `unit_grants_follow_the_ability_switches_at_once`: delete `('viewer','view_grades')` from `org.unit_role_abilities`; Pat on A now has a row with an empty array (a grant with no switches still means view).
- [ ] **Step 2: Run** `cargo test -p server --test read_views unit_` → FAIL (relation `org.units` does not exist).
- [ ] **Step 3: Write the migration.** `org.unit_subtree`: recursive CTE starting from every unit as its own root, descending through `parent_id`, with `CYCLE unit_id SET is_cycle USING path` and a filter on `NOT is_cycle`. `org.unit_grants`: `unit_roles` join `unit_subtree` on `root_id = unit_id`, left join `unit_role_abilities` on role, group by `(user_id, subtree.unit_id)`, `coalesce(array_agg(DISTINCT ability) FILTER (WHERE ability IS NOT NULL), '{}')`. Add both views to `DECLARED_VIEWS` with the comment `// Units and unit roles (ADR-0094, migration 0055).`
- [ ] **Step 4: Run** the migration, then the three tests → PASS; `cargo test -p server --test architecture` → PASS.
- [ ] **Step 5: Commit** `feat(org): units, unit roles and their declared views (ADR-0094)`.

### Task 2: The vocabulary and the rules in `access`

**Files:**
- Create: `backend/crates/access/src/unit.rs` (types below, re-exported from `lib.rs`)
- Modify: `backend/crates/access/src/lib.rs` (`CourseAccess`, `for_actor`, `Action`, `authorize`, `written_course`, new free functions), `backend/crates/access/src/tests.rs`, every `CourseAccess { .. }` literal in the workspace (add `unit_grant: None`), `access/src/plugin.rs` if it builds one.

**Interfaces:**
- Produces:
  ```rust
  pub enum UnitRole { Manager, Viewer }                    // text_enum: "manager", "viewer"
  pub enum UnitAbility { ViewGrades, EditContent, Grade, ManageRoster, ManageCourses, ManageUnits }
                                                           // text_enum: the six strings above
  impl UnitAbility { pub const ALL: [UnitAbility; 6]; }
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct UnitGrant { pub unit_id: Uuid, /* private bitset */ }
  impl UnitGrant {
      pub fn new(unit_id: Uuid, abilities: impl IntoIterator<Item = UnitAbility>) -> Self;
      pub fn full(unit_id: Uuid) -> Self;                  // every ability (an admin's view in GET /units)
      pub fn has(self, ability: UnitAbility) -> bool;
      pub fn abilities(self) -> Vec<UnitAbility>;          // in ALL order
  }
  pub struct CourseAccess { pub role, pub is_owner, pub status, pub unit_grant: Option<UnitGrant> }
  pub enum Via { Admin, Unit }                             // text_enum: "admin", "unit"
  /// How the actor reached the course without being on it, or None when they are on it.
  pub fn implicit_access(actor: &Actor, access: CourseAccess) -> Option<Via>;
  pub struct CourseCan { pub view_grades: bool, pub edit_content: bool, pub grade: bool,
                         pub manage_roster: bool, pub manage_course: bool }
  pub fn course_can(actor: &Actor, access: CourseAccess) -> CourseCan;
  pub fn unit_role_allowed(institution: Role) -> bool;    // Teacher | Admin
  // New Action variants:
  ViewOwnUnits, ViewUnit(Option<UnitGrant>), ManageUnitCourses(Option<UnitGrant>),
  ManageUnits(Option<UnitGrant>), ManageUnitRoleAbilities,
  ```
- `implicit_access`: `Some(Admin)` when the actor is an admin; else `Some(Unit)` when `unit_grant.is_some() && !is_owner && role.is_none()`; else `None`.
- `course_can`: each field is `authorize(actor, X(access)).is_ok()` for `ViewGradebook`, `EditContent`, `GradeAssignment`, `ManageRoster`, `EditCourse` respectively (so an archived course reads as `false` for writes, as the API would answer).
- `implicit_course_owner` stays (it decides that an admin's new course gets no membership).

- [ ] **Step 1: Write the failing tests** in `access/src/tests.rs`, with a teacher actor, `role: None`, `is_owner: false`, a published course:
  - `a_unit_grant_lets_through_what_its_abilities_allow`: one assertion per row of spec section 5's table, a grant with only the named ability → `Ok`, the same grant without it → `Denied` (view rows: any grant, even with no abilities).
  - `a_unit_grant_never_deletes_transfers_or_acts_as_a_student`: `UnitGrant::full` → `Denied` for `DeleteCourse`, `TransferOwnership`, `SubmitWork`, `MarkDone`, `ViewOwnGrades`, `ViewOwnSubmission`.
  - `a_student_holds_no_unit_grant`: a student actor with `UnitGrant::full` → `Denied` for `ViewCourse` of a draft and `EditContent`.
  - `unit_actions_need_the_grant_or_admin`: `ViewUnit(None)` denied for a teacher, allowed for an admin; `ManageUnits(Some(grant without ManageUnits))` denied; `ManageUnitRoleAbilities` admin only; `ViewOwnUnits` allowed for a student.
  - `an_archived_course_refuses_unit_writes`: `UnitGrant::full`, archived → `EditContent` is `Err(Archived)`, `ViewCourse` is `Ok`.
  - `implicit_access_names_the_way_in`: admin owner → `Some(Admin)`; teacher with grant, no role → `Some(Unit)`; course teacher who also has a grant → `None`.
  - `unit_role_allowed_is_for_teachers_and_admins`.
- [ ] **Step 2: Run** `cargo test -p access` → FAIL (types missing).
- [ ] **Step 3: Implement.** In `authorize`, OR a small helper `unit(access, ability) -> bool` into each rule named in the spec table; `for_actor` sets `unit_grant: None` for a student; `written_course` lists nothing new (the unit actions carry no `CourseAccess`). Add `unit_grant: None` to every literal so the workspace builds.
- [ ] **Step 4: Run** `cargo test -p access` → PASS; `cargo check --workspace --all-targets --all-features` → clean.
- [ ] **Step 5: Commit** `feat(access): unit roles, abilities and grants in the rules (ADR-0094)`.

### Task 3: Audit keyed on the access

**Files:**
- Modify: `backend/crates/events/src/service.rs` (`audit_admin` → `audit_implicit`), its 15 callers: `learning/teaching/src/service/{mod.rs,info.rs,announcements.rs}`, `learning/content/src/service.rs`, `learning/gradebook/src/service/groups.rs`
- Test: `backend/crates/events/src/service/tests.rs` (or the crate's existing test file for `audit`)

**Interfaces:**
- Consumes: `access::implicit_access`, `Via` (Task 2).
- Produces: `pub async fn audit_implicit(conn: &mut PgConnection, actor: &Actor, access: CourseAccess, entry: AuditEntry) -> Result<(), EventsError>`: writes the row when `implicit_access` is `Some`, adding `meta.via` (`"admin"`/`"unit"`) and, for `unit`, `meta.unit_id`. `audit_admin` is deleted.

- [ ] **Step 1: Write the failing tests:** `audit_implicit_writes_for_an_admin_with_via_admin`, `audit_implicit_writes_for_a_unit_grant_with_via_and_unit_id`, `audit_implicit_writes_nothing_for_a_course_teacher` (count rows in `events.audit_log` in a `db_test`).
- [ ] **Step 2: Run** them → FAIL.
- [ ] **Step 3: Implement** `audit_implicit`, delete `audit_admin`, change each caller to pass the `CourseAccess` it authorized with. Where a function authorized without keeping the access (`load(..).await?` discarding `.1`), keep it. In `create_course`/`insert_owned_course`, build the access of the new course (`is_owner: true`, `role` the membership just written or `None`, `unit_grant` from Task 6 or `None` until then).
- [ ] **Step 4: Run** the three tests, then `cargo test -p teaching`, `-p content`, `-p gradebook` one at a time → PASS (existing admin-audit tests keep their assertions, plus `via`).
- [ ] **Step 5: Commit** `refactor(events): course changes are audited by how the actor got in (ADR-0094)`.

### Task 4: Units in `org`

**Files:**
- Modify: `backend/crates/org/src/{model.rs,repo.rs,service.rs,lib.rs}`, `org/src/routes/mod.rs`
- Create: `backend/crates/org/src/routes/units.rs`
- Modify: `backend/crates/api-types/src/` (unit DTOs beside the group ones)
- Test: `backend/crates/org/src/service/tests.rs`

**Interfaces:**
- Consumes: `access::{UnitGrant, UnitAbility, Action::{ViewOwnUnits, ViewUnit, ManageUnits}}`.
- Produces (re-exported from `org`):
  ```rust
  pub struct Unit { pub id: Uuid, pub parent_id: Option<Uuid>, pub name: String, pub created_at: DateTime<Utc> }
  pub struct UnitUpdate { pub name: Option<OrgName>, pub parent_id: Option<Option<Uuid>> }
  /// The actor's grant on a unit; None when they hold none. UnitNotFound if the unit does not exist.
  pub async fn unit_grant(conn, actor: &Actor, unit_id: Uuid) -> Result<Option<UnitGrant>, OrgError>;
  pub async fn list_units(conn, actor) -> Result<Vec<(Unit, UnitGrant)>, OrgError>;   // admin: all, UnitGrant::full
  pub async fn create_unit(conn, actor, name: OrgName, parent_id: Option<Uuid>) -> Result<Unit, OrgError>;
  pub async fn update_unit(conn, actor, id: Uuid, update: UnitUpdate) -> Result<Unit, OrgError>;
  pub async fn delete_unit(conn, actor, id: Uuid) -> Result<(), OrgError>;
  ```
- New `OrgError` variants: `UnitNotFound` (404), `UnitNameTaken` (409, as `GroupNameTaken`), `UnitCycle` (422 on `parent_id`), `UnitInUse` (409: subunits or courses; mapped from the foreign-key violation like `TermInUse`).
- Routes (spec section 7): `GET /units`, `POST /units`, `PATCH /units/{id}`, `DELETE /units/{id}`. `GET /units` items: `{id, parent_id, name, abilities: [..]}`.
- Rule for `ManageUnits`: the grant checked is the one on the **parent** (`None` parent → `ManageUnits(None)`, admin only). A move checks the old parent and the new one. A unit the actor cannot `ViewUnit` is `UnitNotFound`.
- Every write audits with `events::audit` (`unit.create`, `unit.rename`, `unit.move`, `unit.delete`; `meta`: ids and the name).
- A move runs `LOCK TABLE org.units IN SHARE ROW EXCLUSIVE MODE` (repo fn `lock_tree`), then refuses a new parent inside `org.unit_subtree` of the unit.

- [ ] **Step 1: Write the failing tests** (`db_test`, teacher Pat `manager` of A, A > B):
  - `a_manager_creates_renames_moves_and_deletes_below_their_unit`: under A succeeds; B renamed; C created under B, moved under A; C deleted.
  - `a_manager_cannot_touch_their_own_unit_or_create_at_the_top`: rename A, delete A, move A, create a top-level unit → `Forbidden`/`UnitNotFound` as the rule says; admin can.
  - `a_unit_cannot_move_under_itself_or_below`: move A under B → `UnitCycle`; move A under A → `UnitCycle`.
  - `a_unit_with_subunits_or_courses_is_not_deleted`: delete A (has B) → `UnitInUse`; a course with `unit_id = B` → delete B is `UnitInUse`.
  - `sibling_names_are_unique_top_level_too`: two top-level "Science" → `UnitNameTaken`; "Science" under A and under B → ok.
  - `list_units_shows_the_covered_subtree_with_abilities`: Pat sees A and B with the manager abilities, not a sibling; admin sees all with every ability; a student sees none.
- [ ] **Step 2: Run** `cargo test -p org unit` → FAIL.
- [ ] **Step 3: Implement** model, repo (SQL incl. `lock_tree`, subtree check, grant read from `org.unit_grants`), service, routes with utoipa, DTOs.
- [ ] **Step 4: Run** `cargo test -p org unit` → PASS.
- [ ] **Step 5: Commit** `feat(org): the unit tree and its endpoints (ADR-0094)`.

### Task 5: Unit roles and their abilities in `org`

**Files:**
- Modify: `backend/crates/org/src/{model.rs,repo.rs,service.rs,lib.rs}`, `org/src/routes/units.rs`, `org/src/routes/mod.rs`; `backend/crates/common/src/` (receive `like_pattern`), `learning/teaching/src/service/info.rs` (use it from `common`)
- Create: `backend/crates/org/src/routes/unit_roles.rs` (the admin abilities routes)
- Modify: `backend/crates/learning/flows/src/people/service.rs`
- Test: `backend/crates/org/src/service/tests.rs`, `learning/flows/src/people/tests.rs` (or its existing test file)

**Interfaces:**
- Consumes: Task 4's `unit_grant`, `UnitNotFound`; `auth::find_user_shared`; `access::{UnitRole, unit_role_allowed, Action::ManageUnitRoleAbilities}`.
- Produces:
  ```rust
  pub struct UnitRoleEntry { pub user_id: Uuid, pub name: String, pub surname: String, pub email: String, pub role: UnitRole }
  pub async fn list_unit_roles(conn, actor, unit_id) -> Result<Vec<UnitRoleEntry>, OrgError>;          // ViewUnit
  pub async fn set_unit_role(conn, actor, unit_id, user_id, role: UnitRole) -> Result<(), OrgError>;   // ManageUnits on parent
  pub async fn remove_unit_role(conn, actor, unit_id, user_id) -> Result<(), OrgError>;                // ManageUnits on parent
  pub async fn unit_candidates(conn, actor, unit_id, q: &str) -> Result<Vec<UnitCandidate>, OrgError>; // ManageUnits on parent
  pub async fn drop_unit_roles(conn, actor, user_id) -> Result<(), OrgError>;     // for the demotion flow; audits each
  pub async fn unit_role_abilities(conn, actor) -> Result<Vec<(UnitRole, Vec<UnitAbility>)>, OrgError>;
  pub async fn set_unit_role_abilities(conn, actor, role: UnitRole, abilities: Vec<UnitAbility>) -> Result<(), OrgError>;
  pub fn like_pattern(q: &str) -> String;   // in common, moved unchanged
  ```
- `set_unit_role` reads the person with `auth::find_user_shared` (row locked `FOR SHARE`) and refuses with a new `OrgError::RoleNotAllowed` (422 on `role`) unless `unit_role_allowed`; unknown user → `MemberNotFound`-style 404 (reuse the existing user-not-found mapping). Idempotent upsert. Audits `unit.role.set` / `unit.role.remove` / `unit.abilities.set` (`meta`: unit id, user id, role, abilities).
- Candidates: active teachers and admins, without a role on this unit, matching name/surname/email like `teaching::search_candidates`, same min length and limit constants (move them to `common` with `like_pattern` if they live beside it).
- Routes: `GET /units/{id}/roles`, `PUT /units/{id}/roles/{user_id}` `{role}`, `DELETE /units/{id}/roles/{user_id}`, `GET /units/{id}/candidates?q=`, `GET /admin/unit-roles` → `{manager: [..], viewer: [..]}`, `PUT /admin/unit-roles/{role}` `{abilities: [..]}` (replace in one transaction; unknown string → 422 via the DTO enum).
- `learning::people::update_user`: on `demoted`, call `org::drop_unit_roles(conn, actor, user_id)` next to `drop_teacher_memberships`.

- [ ] **Step 1: Write the failing tests:**
  - `a_student_cannot_hold_a_unit_role`: `set_unit_role` for a student → `RoleNotAllowed`.
  - `a_manager_grants_below_but_not_on_their_own_unit`: Pat (manager of A) sets Quinn `viewer` of B → ok; sets Quinn on A → denied; removes their own role on A → denied.
  - `switching_an_ability_off_changes_the_grant_at_once`: `set_unit_role_abilities(Viewer, [])` then `unit_grant` of a viewer has no `ViewGrades` but is `Some`.
  - `only_an_admin_sets_role_abilities`: Pat → `Forbidden`.
  - `candidates_are_active_teachers_and_admins_not_yet_on_the_unit`.
  - `a_demoted_teacher_loses_their_unit_roles` (flows test): teacher with a role on A demoted to student → no `org.unit_roles` row, one `unit.role.remove` audit row.
- [ ] **Step 2: Run** them one at a time → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS; `cargo test -p teaching info` → PASS (candidate search after the move of `like_pattern`).
- [ ] **Step 5: Commit** `feat(org): unit roles and the abilities admins switch (ADR-0094)`.

### Task 6: `teaching` reads the grant

**Files:**
- Modify: `backend/crates/learning/teaching/src/{model.rs,repo/mod.rs,service/mod.rs,routes/mod.rs,lib.rs}`, the template create path (`POST /templates/{id}/courses`), `backend/crates/api-types/src/lib.rs` (`Course.unit_id`, `CreateCourseRequest.unit_id`, the template request's `unit_id`, `CourseDetail`, `CourseCan`, `SetCourseUnitRequest`)
- Test: `backend/crates/learning/teaching/src/service/tests.rs`

**Interfaces:**
- Consumes: `org::unit_grant`, `access::{UnitGrant, course_can, Action::{ManageUnitCourses, ViewUnit}}`, view `org.unit_grants`, view `org.unit_subtree`.
- Produces:
  - `Course.unit_id: Option<Uuid>` (domain and API); `teaching.course_list` is not changed (no reader needs it yet).
  - `CourseRow`/`find_course`/`list_courses_of` select `g.abilities AS "unit_abilities?"` from `LEFT JOIN org.unit_grants g ON g.user_id = $2 AND g.unit_id = c.unit_id`; the locked path reads it in a second statement after `lock_course` (`repo::find_unit_grant(conn, unit_id, user_id) -> Option<Vec<String>>`). Parse strings with `UnitAbility::parse`; an unknown string is a `sqlx::Error::Decode`-style error, never ignored.
  - `access_of(actor, course, role, unit_grant: Option<UnitGrant>)`.
  - `create_course(conn, actor, title, term_id, unit_id: Option<Uuid>)` and the template create: with a unit, `org::unit_grant` then `authorize(ManageUnitCourses(grant))` before inserting.
  - `pub async fn set_course_unit(conn, actor, course_id, unit_id: Option<Uuid>) -> Result<Course, TeachingError>`: `load(lock)`, then `ManageUnitCourses` on the old unit (if any) and on the new (if any); no `EditCourse`, so an archived course moves. Audits `course.unit` via `audit_implicit`. Route `PUT /courses/{id}/unit`.
  - `pub async fn list_unit_courses(conn, actor, unit_id, page: PageParams) -> Result<Page<Course>, TeachingError>`: `ViewUnit(org::unit_grant(..))`, courses whose `unit_id` is in `org.unit_subtree` of `unit_id`, every status. Route `GET /units/{id}/courses` (in `teaching`'s router).
  - `GET /courses/{id}` returns `CourseDetail { #[serde(flatten)] course: Course, can: CourseCan }`.

- [ ] **Step 1: Write the failing tests** (Pat `viewer` of A, course K in B under A, Pat not on K):
  - `a_unit_viewer_sees_a_course_below_in_every_status`: `get_course` of draft K → ok; `course_access(..).unit_grant` is `Some`.
  - `a_unit_viewer_cannot_edit_and_a_manager_with_edit_content_can`: `update_course` → `Forbidden`; switch `edit_content` on for viewer → ok.
  - `creating_in_a_unit_needs_manage_courses`: teacher without a grant → `Forbidden`; manager → ok, `unit_id` set.
  - `an_archived_course_moves_between_units`: archive K, `set_course_unit(K, Some(A))` by an admin → ok.
  - `moving_needs_the_grant_on_both_units`: manager of B only, move K to a sibling S → `Forbidden`.
  - `list_unit_courses_covers_the_subtree`: list A → [K]; list of a sibling → []; teacher without a grant → `UnitNotFound`-style not found.
  - `can_tells_a_viewer_what_they_may_do`: `course_can` for the viewer: `view_grades: true`, the rest `false`.
- [ ] **Step 2: Run** `cargo test -p teaching unit` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p teaching unit` → PASS; `cargo test -p teaching --no-run`, and the same for `content`, `assessment`, `gradebook`, `learning` → build clean.
- [ ] **Step 5: Commit** `feat(teaching): courses in units, read through the unit grant (ADR-0094)`.

### Task 7: The API end to end and the authorization list

**Files:**
- Create: `backend/crates/server/tests/units.rs`
- Modify: `backend/crates/server/tests/authorization.rs` (`Ctx` gets `unit`; `operations` lists every new operation; `is_institutional` adds `/units` only if the test's semantics need it, after reading how it is used), `backend/crates/server/tests/common/mod.rs` (helpers `create_unit(http, token, name, parent) -> Uuid`, `set_unit_role(http, token, unit, user, role)` if more than one test needs them)

**Interfaces:**
- Consumes: every route of Tasks 4–6.

- [ ] **Step 1: Write the failing API tests** in `units.rs` (real server, `reqwest`):
  - `a_manager_runs_a_course_of_their_unit_without_being_on_it`: admin makes A > B, makes Pat `manager` of A; teacher Tom creates K and admin puts it in B (`PUT /courses/{K}/unit`); Pat: `GET /courses/{K}` 200 with `can.manage_roster == true`, `can.edit_content == false`; `POST /courses/{K}/publish` 200; `PATCH /courses/{K}` title 200; `DELETE /courses/{K}` 403; `POST /courses/{K}/owner` 403.
  - `a_managers_change_is_audited_and_the_teachers_is_not`: after Pat publishes, `events.audit_log` has a `course.publish` row with `meta.via = "unit"` and `meta.unit_id = B`; Tom's `PATCH` title adds no row.
  - `switching_an_ability_off_takes_effect_on_the_next_request`: admin `PUT /admin/unit-roles/viewer {abilities: []}`; viewer Vic's `GET /courses/{K}/gradebook` goes 200 → 403 (or 404 per the existing mapping).
  - `revoking_a_role_takes_effect_on_the_next_request`: `DELETE /units/{A}/roles/{Pat}`; Pat's `GET /courses/{K}` → 404.
  - `a_student_is_refused_a_unit_role`: `PUT /units/{A}/roles/{student}` → 422 on `role`.
  - `a_sibling_unit_sees_nothing`: Sam `manager` of S; `GET /courses/{K}` → 404; `GET /units/{B}/courses` → 404.
- [ ] **Step 2: Add the new operations to `authorization.rs`** (each with a valid body): the 4 unit routes, the 4 role routes, the 2 admin ability routes, `GET /units/{id}/courses`, `PUT /courses/{id}/unit`. Run `cargo test -p server --test authorization the_list_of_operations_is_the_api` → FAIL before the list is complete, PASS after; then `no_operation_answers_a_caller_who_may_not_use_it` → PASS.
- [ ] **Step 3: Run** `cargo test -p server --test units` → PASS (fix what fails in the owning task's code).
- [ ] **Step 4: Commit** `test(server): units over HTTP and in the authorization list (ADR-0094)`.

### Task 8: Prepared queries, OpenAPI, docs

**Files:**
- Modify: `backend/.sqlx/`, the checked-in OpenAPI document (wherever `cargo test -p server --test api` compares it), `docs/decisions/in-progress.md` (ADR-0094 line), `docs/PLAN.md` / `ARCHITECTURE.md` (org now owns units; the layer line in `CLAUDE.md` §4 `org  terms, groups, subgroups, units and unit roles, the installation record`; the declared-view list in §4 gains `org.unit_subtree`, `org.unit_grants`), `CHANGELOG.md` (Unreleased).

- [ ] **Step 1:** `cargo sqlx prepare --workspace -- --all-targets --all-features` from `backend/`; `git status backend/.sqlx` shows only additions/updates.
- [ ] **Step 2:** Regenerate OpenAPI the way the repo does (find the command in `docs/development.md`); `cargo test -p server --test api` → PASS.
- [ ] **Step 3:** Write the ADR-0094 line in `in-progress.md` in the style of ADR-0092's: what is built (everything in this plan), what is missing (the web pages, PR B). Update the docs listed above.
- [ ] **Step 4: Definition of done** (CLAUDE.md §14) from `backend/`, one at a time with `CARGO_BUILD_JOBS=2`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo sqlx prepare --workspace --check -- --all-targets --all-features`; `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; the targeted tests of Tasks 1–7. The full suite runs in CI.
- [ ] **Step 5: Commit** `docs: org units (ADR-0094)`; stop for review before opening the PR.
