-- ADR-0089. How a course turns points into a result: a target (usually 100) and a pass mark, and
-- groups of assignments that can have a cap, count only the best K, count as practice only, or be
-- bonus. A course shows its groups in one of two layouts: `course` (a group stands for a top-level
-- module) or `custom` (the teacher's own groups). Like gradebook.grades, none of these rows has a
-- foreign key into another schema (ADR-0021): content and teaching call the gradebook's purge
-- before they delete a module, an assignment or a course.

CREATE TABLE gradebook.course_settings (
    course_id  uuid         PRIMARY KEY,
    target     numeric(6,2) NOT NULL DEFAULT 100 CHECK (target > 0),
    pass_mark  numeric(6,2) CHECK (pass_mark >= 0),
    layout     text         NOT NULL DEFAULT 'course' CHECK (layout IN ('course', 'custom')),
    updated_at timestamptz  NOT NULL DEFAULT now(),
    CHECK (pass_mark IS NULL OR pass_mark <= target)
);

CREATE TABLE gradebook.groups (
    id        uuid         PRIMARY KEY,
    course_id uuid         NOT NULL,
    -- Set for a group that stands for a top-level module (layout `course`); the module's own title
    -- and place are used then. NULL for a group of the teacher's own (layout `custom`).
    module_id uuid,
    title     text         CHECK (title = btrim(title) AND char_length(title) BETWEEN 1 AND 200),
    position  integer      CHECK (position >= 1),
    -- The most the group adds to the course total; NULL = no cap.
    cap       numeric(6,2) CHECK (cap > 0),
    -- Only the K best marks count; NULL = all of them.
    best_k    integer      CHECK (best_k >= 1),
    -- False = practice: graded and shown, but not part of the total.
    counts    boolean      NOT NULL DEFAULT true,
    -- True = extra: adds to the total, not to what a student can still reach.
    bonus     boolean      NOT NULL DEFAULT false,
    CHECK ((module_id IS NULL) = (title IS NOT NULL)),
    CHECK ((module_id IS NULL) = (position IS NOT NULL)),
    CHECK (counts OR NOT bonus)
);
CREATE UNIQUE INDEX groups_module_key ON gradebook.groups (module_id) WHERE module_id IS NOT NULL;
CREATE INDEX groups_course_idx ON gradebook.groups (course_id);

-- Which custom group an assignment sits in. An assignment with no row is in "Not sorted".
-- Deleting a group sends its assignments back there.
CREATE TABLE gradebook.group_items (
    assignment_id uuid PRIMARY KEY,
    group_id      uuid NOT NULL REFERENCES gradebook.groups (id) ON DELETE CASCADE
);
CREATE INDEX group_items_group_idx ON gradebook.group_items (group_id);
