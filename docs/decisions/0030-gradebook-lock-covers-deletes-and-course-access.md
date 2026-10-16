# 0030. The assignment-row lock covers deletes; the gradebook may ask teaching for course access

Status: Accepted
Amends: ADR-0028

## Context
ADR-0028 is merged, so it is an accepted record and is not edited. Two of its points turned out narrower than the code needs.

Point 5 serializes a grade write with a change of `max_points` through the assignment row in `content.assignments`. The delete guard of point 9 needs the same serialization: a delete that checks for grades and then removes the assignment, while a grade write is in flight, would leave a grade with no assignment.

Point 8 says `gradebook` never calls the Rust functions of `content`, `teaching` or `identity`. To authorize, the gradebook has to know how the actor relates to the course: role, owner, status. `content` gets that from `teaching::course_access`, the one read function of the course-level rules. A second query in `gradebook` would repeat the membership and staff rules, and when `effective_members` and groups arrive there would be two places to change.

## Options
- Point 5: (a) leave deletes unlocked, so the guard can race with a grade write; (b) **the same row lock for deletes**.
- Point 8: (a) keep the course-access query in `gradebook/repo.rs`: two copies of the rules; (b) **allow the read-only course-access function of `teaching`**; (c) move course access into a neutral module: a new abstraction for one function.

## Decision
- Point 5 reads: a grade write, a change of `max_points` and a delete of that assignment are serialized through the assignment row in `content.assignments`. The grade write and the publish take it `FOR SHARE`, and read `max_points` from the locked row. The `max_points` update and the deletes (of the assignment, of a module that contains it, and later of a course) take it `FOR UPDATE` first and call the gradebook in a later statement. A module delete locks its assignments in id order.
- Point 8 reads: `gradebook` calls no other module's Rust code, except the read-only function of `teaching` that resolves a user's access to a course (`teaching::course_access`, and any read-only function that replaces it for the same purpose) which every module uses to authorize. Everything else in point 8 stands: `content`, `teaching` and `identity` tables are read with read-only JOINs only, and a `FOR SHARE` lock on `content.assignments` is a lock, not a write.

## Consequences
- Today the module-level call graph has no cycle: `content` calls `teaching` and `gradebook`, `gradebook` calls `teaching`, `teaching` calls neither. The task that adds course deletion makes `teaching` call `gradebook` (`has_grades_in_course`, `purge_course`), and then `teaching` and `gradebook` call each other. Rust allows that inside one crate and it compiles; it is a real cycle and is to be noticed then, not hidden behind an abstraction.
- A delete waits for a grade write in flight on the same assignment, and the other way round.
- `gradebook` has no copy of the course-access rules. The roster queries (`teaching.memberships` JOINs for the current students) remain in `gradebook/repo.rs` as read models and are the place that changes when `effective_members` arrives.
