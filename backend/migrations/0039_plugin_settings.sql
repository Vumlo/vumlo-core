-- Plugin settings (ADR-0072). A plugin declares what an administrator may configure in its manifest;
-- the declaration is kept with the install, the values in their own table. No plugin is installed
-- yet that this could break: an install made before this migration declares no settings.

ALTER TABLE plugins.plugin_installs
    ADD COLUMN settings jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(settings) = 'object');

-- One value of one setting of one install, for the whole institution (`course_id` is NULL) or for
-- one course. A secret is stored sealed (a base64url string), never as it was typed. A value that is
-- not here is the default of the setting, or nothing when it has none.
CREATE TABLE plugins.settings (
    install_id uuid        NOT NULL REFERENCES plugins.plugin_installs (id) ON DELETE CASCADE,
    course_id  uuid        REFERENCES teaching.courses (id) ON DELETE CASCADE,
    key        text        NOT NULL CHECK (key ~ '^[a-z][a-z0-9_]{0,39}$'),
    value      jsonb       NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- At most one value per setting and scope; the nil uuid stands for "no course" because NULLs differ.
CREATE UNIQUE INDEX settings_one_value
    ON plugins.settings (install_id, key, COALESCE(course_id, '00000000-0000-0000-0000-000000000000'));
CREATE INDEX settings_by_course ON plugins.settings (course_id) WHERE course_id IS NOT NULL;
