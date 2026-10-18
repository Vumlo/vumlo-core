-- ADR-0031. The installation record: one row, written by the first start. It holds the institution
-- mode, which never changes in v1; every later start compares the configuration with it.
CREATE TABLE org.installation (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    mode      text    NOT NULL CHECK (mode IN ('school', 'university'))
);
