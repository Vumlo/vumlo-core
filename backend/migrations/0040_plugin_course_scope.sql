-- Course scoping of plugins (ADR-0074). Until now a plugin with a scope read every course. From now an
-- administrator grants a plugin all courses, or an explicit list, and every plugin call is checked
-- against it.

ALTER TABLE plugins.plugin_installs ADD COLUMN all_courses boolean NOT NULL DEFAULT false;

-- What an install could do before this migration is what `all_courses` means: nothing is narrowed
-- behind an administrator's back.
UPDATE plugins.plugin_installs SET all_courses = true;

-- The courses an install may reach when `all_courses` is false. Deleting a course drops it from
-- every list.
CREATE TABLE plugins.install_courses (
    install_id uuid NOT NULL REFERENCES plugins.plugin_installs (id) ON DELETE CASCADE,
    course_id  uuid NOT NULL REFERENCES teaching.courses (id) ON DELETE CASCADE,
    PRIMARY KEY (install_id, course_id)
);

CREATE INDEX install_courses_by_course ON plugins.install_courses (course_id);
