-- ADR-0093. The draft grade of a quiz is written by a subscriber of `quiz.attempt_submitted`, in the
-- transaction of the submit, not by a call from `assessment` into `gradebook`. The subscriber finds
-- the sum on the attempt itself: set only when every question was scored (ADR-0050 point 5), empty
-- when a person has to grade some of it.
ALTER TABLE assessment.submissions
    ADD COLUMN auto_total numeric(6,2) CHECK (auto_total >= 0);
