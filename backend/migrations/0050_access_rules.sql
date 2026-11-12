-- ADR-0092. Completion and access rules. A module or an item can be locked until the student has
-- done something: finished an item or a whole module, or reached a score on an assignment. Locked
-- is not hidden: the title stays in the course tree, and the body, the files and the right to
-- submit wait for the rule.
--
-- A rule is rows, not a JSON value, so that a condition cannot outlive what it names: deleting an
-- item or a module deletes the conditions that point at it, and the nodes they lock lose the lock
-- instead of being locked for good.

-- A student's own mark that a text or file item is done. Assignments need no mark: they are done
-- by what the student submits or is graded (see `learning::content::rules`).
CREATE TABLE content.completions (
    item_id    uuid        NOT NULL REFERENCES content.items (id) ON DELETE CASCADE,
    student_id uuid        NOT NULL REFERENCES auth.users (id),
    at         timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (item_id, student_id)
);

CREATE TABLE content.access_conditions (
    id               uuid         PRIMARY KEY,
    course_id        uuid         NOT NULL,
    -- The node that is locked: exactly one of an item and a module.
    item_id          uuid         REFERENCES content.items (id) ON DELETE CASCADE,
    module_id        uuid         REFERENCES content.modules (id) ON DELETE CASCADE,
    -- The order the teacher gave the conditions of one node.
    position         integer      NOT NULL CHECK (position >= 1),
    kind             text         NOT NULL CHECK (kind IN ('completed', 'score_at_least')),
    -- What the condition is about: `completed` an item or a module, `score_at_least` an assignment.
    target_item_id   uuid         REFERENCES content.items (id) ON DELETE CASCADE,
    target_module_id uuid         REFERENCES content.modules (id) ON DELETE CASCADE,
    points           numeric(6,2) CHECK (points > 0),
    CHECK ((item_id IS NULL) <> (module_id IS NULL)),
    CHECK (CASE kind
        WHEN 'completed' THEN points IS NULL AND ((target_item_id IS NULL) <> (target_module_id IS NULL))
        ELSE points IS NOT NULL AND target_item_id IS NOT NULL AND target_module_id IS NULL
    END)
);

-- All the conditions of a course are read together, for the tree.
CREATE INDEX access_conditions_course_idx ON content.access_conditions (course_id);
-- One position per node, so a rule is a list.
CREATE UNIQUE INDEX access_conditions_item_position_idx
    ON content.access_conditions (item_id, position) WHERE item_id IS NOT NULL;
CREATE UNIQUE INDEX access_conditions_module_position_idx
    ON content.access_conditions (module_id, position) WHERE module_id IS NOT NULL;
