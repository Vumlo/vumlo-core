# 0039. Input the database cannot store, races, a busy database, unreadable bodies

Status: Accepted
Amends: ADR-0012, ADR-0018, ADR-0020

## Context
The audit of task 014 found places where a request that is not the server's fault ends as `500`, and
places where the documents and the code disagree about an error:

- A-07: a NUL character in the text of a document (a student's autosaved draft included), or a term date
  before 4713 BC, passes validation and fails in PostgreSQL.
- A-08: a `PATCH` of a module or item that is deleted while the request waits for the row lock failed
  with `RowNotFound` (`500`); a `DELETE` that lost the race answered `204` and wrote an audit row for a
  delete that did not happen.
- A-11: a pool that has no connection to give within its timeout, or a database that is unreachable,
  was a bare `500` everywhere but `/health`, although `app.rs` said it would be `503`.
- A-14: ADR-0018 says a body that cannot be read is `422`; the code answered `400`. ADR-0020 says the
  positions of siblings are dense; after a delete they had a gap. ADR-0012 promises an
  `Idempotency-Key` that no endpoint accepts and AGENTS §9 forbids to add.

## Decision
1. **What the database cannot store is refused at the edge.** Text in a document (a run, a code or
   formula source, inline formula, link text) may hold no control character other than a line feed, a
   carriage return and a tab: `422` on the path of the text (`…content[0].text`), message `contains a
   control character`. A term date must lie in the years 1 to 9999: `422` on `starts_on` or `ends_on`.
2. **A write by id that finds its row gone is `404`.** It happens when the request waited for a lock
   and the holder deleted the row. Such a request changes nothing and writes no audit row. The same
   holds for a publish. (`update_*` return the row or nothing; `delete_*` check the row count.)
3. **A database that cannot serve the request is `503` with `Retry-After: 5`.** The pool timeout, a
   closed pool, a lost connection, SQLSTATE class 08 (connection), 53 (resources, too many connections)
   and 57P (shutdown or restart) map to it, in every module, through `ApiError::from_db`. Any other
   database error is a logged `500` as before.
4. **A body that cannot be read is `422`** (invalid JSON, a value of the wrong type, nesting beyond the
   parser's depth), with a fixed `detail` that does not repeat what was sent. A missing or wrong media
   type is `415` and a body that is too large `413`. A malformed path parameter stays `400`. ADR-0018
   is read this way.
5. **Positions are dense.** A delete renumbers the siblings that remain, in the same transaction
   (`1, 2, 3 …`, in their order; `updated_at` is not touched). Two deletes in the same sequence at the
   same instant may leave a gap that the next reorder closes; a create racing a delete conflicts at
   commit and is retried (`409`, ADR-0020).
6. **`Idempotency-Key` is deferred.** ADR-0012's sentence stands as a goal and is not met by any
   endpoint yet; a retry-safe design needs its own table and task. No endpoint reads the header, and none
   may get ad-hoc handling.
7. **A cursor is scoped to what it pages.** The cursor of the attempts list counts only if it is an
   attempt of that student at that assignment; any other id gives an empty page instead of an ordering
   decided by someone else's row.

## Consequences
- A client can retry a `503`; it never has to treat a `500` as "maybe".
- A `409` on a delete of something already deleted becomes `404`; clients that treated both as "gone"
  lose nothing.
- Existing stored documents are not rescanned. Nothing could hold a NUL; other control characters that
  were stored before stay.
