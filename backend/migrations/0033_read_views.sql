-- What a layer above may read of the layers below it (ADR-0065). The owner declares the columns;
-- nothing above reads the table itself. Plain views on one table, so Postgres reads through them
-- and row locks taken through them lock the table's rows.

-- Who a person is, for a list or a screen that names them. No password, no session, no flag.
CREATE VIEW auth.people AS
SELECT id, name, surname, email, role, status FROM auth.users;

CREATE VIEW org.term_list AS
SELECT id, name, starts_on, ends_on FROM org.terms;

CREATE VIEW org.group_list AS
SELECT id, name FROM org.groups;

CREATE VIEW org.subgroup_list AS
SELECT id, group_id, name FROM org.subgroups;

-- The audit log as `learning` reads it for the administrator's report.
CREATE VIEW events.audit_entries AS
SELECT id, at, source, action, resource_type, resource_id, meta, actor_id FROM events.audit_log;
