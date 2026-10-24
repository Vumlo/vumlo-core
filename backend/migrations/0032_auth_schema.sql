-- The identity module is now `auth` (ADR-0065). Renaming the schema keeps every table, row,
-- constraint, index and foreign key that points into it.
ALTER SCHEMA identity RENAME TO auth;
