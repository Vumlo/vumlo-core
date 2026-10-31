-- Which files an owner uses (ADR-0069). The owning service rewrites its rows in the same
-- transaction as every write of the body or content; `learning::files` reads them to decide who may
-- read a file and which files nothing uses. `file_id` has no foreign key: `files` is another
-- schema, below this one. Deleting the item or the submission removes its rows.

-- An item's working copy (`published = false`) and its published copy (`published = true`) name
-- their files separately: a file removed from the working copy is still served to students until
-- the item is published again.
CREATE TABLE content.item_files (
    item_id   uuid    NOT NULL REFERENCES content.items (id) ON DELETE CASCADE,
    file_id   uuid    NOT NULL,
    published boolean NOT NULL,
    PRIMARY KEY (item_id, file_id, published)
);
CREATE INDEX item_files_file_idx ON content.item_files (file_id);

-- The files of a draft or of a submitted attempt (a submit turns the draft row into the attempt in
-- place, so the rows stay).
CREATE TABLE assessment.submission_files (
    submission_id uuid NOT NULL REFERENCES assessment.submissions (id) ON DELETE CASCADE,
    file_id       uuid NOT NULL,
    PRIMARY KEY (submission_id, file_id)
);
CREATE INDEX submission_files_file_idx ON assessment.submission_files (file_id);
