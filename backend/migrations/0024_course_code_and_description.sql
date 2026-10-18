-- ADR-0048. A course's short code ("CS 305") and a description for its header.
ALTER TABLE teaching.courses
    ADD COLUMN code        text NOT NULL DEFAULT ''
        CONSTRAINT courses_code_check CHECK (code = btrim(code) AND char_length(code) <= 32),
    ADD COLUMN description text NOT NULL DEFAULT ''
        CONSTRAINT courses_description_check CHECK (description = btrim(description) AND char_length(description) <= 300);
