-- ADR-0021. An item is a lesson or an assignment; an assignment has exactly one row here.
ALTER TABLE content.items
    DROP CONSTRAINT items_kind_check,
    ADD CONSTRAINT items_kind_check CHECK (kind IN ('lesson', 'assignment')),
    ADD CONSTRAINT items_id_kind_key UNIQUE (id, kind);

CREATE TABLE content.assignments (
    item_id    uuid         PRIMARY KEY,
    -- Validated in code through the AssignmentType trait, so a new type needs no migration (ADR-0008).
    type       text         NOT NULL,
    config     jsonb        NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(config) = 'object'),
    max_points numeric(6,2) NOT NULL CHECK (max_points > 0),
    is_bonus   boolean      NOT NULL DEFAULT false,
    created_at timestamptz  NOT NULL DEFAULT now(),
    updated_at timestamptz  NOT NULL DEFAULT now(),
    -- The row belongs to an item of kind 'assignment': item_kind is always 'assignment', so the
    -- foreign key below only matches such an item (the pattern of the depth rule in 0005). It must
    -- be STORED: a virtual generated column cannot be part of a foreign key.
    item_kind  text         NOT NULL GENERATED ALWAYS AS ('assignment') STORED,
    FOREIGN KEY (item_id, item_kind) REFERENCES content.items (id, kind) ON DELETE CASCADE
);
