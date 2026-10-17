# 0034. Admin writes outside a course are audited

Status: Accepted
Supplements: ADR-0004

## Context
ADR-0004 audits every admin action inside a course. The admin API of `identity` (create a user, change a role, disable an account, reset a password) changes who may do what in the whole institution and was not audited. Task 014 audited the admin writes of `org`, so `identity` is the gap. The server CLI (`create-admin`, `create-user`) creates accounts too, with no signed-in user, and `events.audit_log.actor_id` is `NOT NULL`.

## Options
- Which admin writes: (a) only role changes; (b) **every write of the admin API of `identity`**.
- One row per request or per change: (a) one `user.update` row per `PATCH` with the fields; (b) **one row per kind of change**, so the action name says what happened and a filter on `user.role_change` finds every promotion.
- The CLI: (a) not audited, because the operator already has the database; (b) **audited with no actor and the source `cli`**, which needs `actor_id` to be nullable and a `source` column. The first admin of an installation is always created this way, which is the row an auditor most wants.

## Decision
1. Every write an admin makes in the admin API of `identity` is written to `events.audit_log` in the same transaction, with the admin as actor, resource type `user` and the target user as the resource. The actions:
   - `user.create`, meta `{"role"}`;
   - `user.rename`, when the name or the surname changed, meta `{"fields": ["name", "surname"]}` (only those that changed);
   - `user.role_change`, meta `{"from", "to"}`;
   - `user.disable` and `user.enable`, meta `{}`; disabling also deletes the user's sessions, which is part of the same action;
   - `user.password_reset`, meta `{}`; it also deletes the user's sessions.
   One `PATCH` that changes several things writes one row for each.
2. What is not audited: reads; a write that is refused or fails (the transaction rolls back, so its row never exists); a `PATCH` or a put that changes nothing; what a user does to their own account (login, logout, their own password change, their own sessions); the expiry of sessions. Failed logins and the like are security logging, not audit, and are not decided here.
3. `meta` holds ids, roles and the names of fields only. It never holds a password, a temporary password, a hash, a token, an email address, or a name or surname.
4. The server CLI is audited the same way where it writes: `create-admin` and `create-user` write `user.create` with no actor and the source `cli`. `events.audit_log` gets a column `source` (`api` or `cli`, default `api`) and `actor_id` becomes nullable, with a check that ties them: an `api` row has an actor and a `cli` row has none. Existing rows are `api` rows and are not changed.
5. `events::audit` keeps its signature (an actor, source `api`); `events::audit_cli` writes the `cli` row. `identity::create_user` stays the unaudited low-level function that tests seed users with; the CLI calls a new `identity::create_user_from_cli`, and the admin API's own function audits.

## Consequences
- Who made a user an admin, who disabled an account, and who reset a password can be answered from the log, and so can how the first admin came to exist.
- `actor_id` can be null, so a reader of the log must not assume an actor; the `source` column says why it is missing.
- A new admin write in `identity` has to say in its review which action it writes. There is no check that forces it; the tests of this ADR cover the operations that exist.
- Not decided: reading the log through the API, retention, and security logging of failed logins.
