# 0060. Desktop offline store and send queue

Status: Accepted
Amends: ADR-0056

## Context
The desktop app must show what a person has read when there is no connection, and keep what a student
writes until there is one. The server has the last word on deadlines and grades (ADR-0035, ADR-0009), and
the app must say so honestly when it refuses something it had accepted into the queue. The scope is
reading (courses, materials, deadlines, grades, the calendar) and drafts with a send queue; quizzes are
not available offline.

## Options
1. **A cache in the web layer** (IndexedDB, or the `localStorage` the web app already uses for small
   settings). The window would own the offline logic, and the token, the queue and the cache would be
   three things JavaScript can reach.
2. **A cache and a queue in Rust** (ADR-0056), beneath the window: the window asks `api(...)` and gets an
   answer, from the server or from the last copy.

## Decision
1. **Option 2.** `desktop/src/store.rs` is one SQLite file (`offline.sqlite3`, `rusqlite` with the bundled
   SQLite) in the app data directory, with the tables `cache`, `outbox` and `meta`. Migrations use
   `PRAGMA user_version`. `lib/query.ts` stays what it was: the in-memory state of the screens. Beneath
   it, the Rust side is the persistent read-through cache.
2. **Reading.** Every `GET` (except `/auth/*` and `/health`) goes to the server with the stored `ETag`
   (ADR-0059). `200` replaces the copy; `304` keeps it and renews its time; a server error or no
   connection serves the copy and says so (`from_cache`, `cached_at`); `403` and `404` delete the copy,
   because the server wins for what teachers publish and for grades: the app never shows what the
   server stopped showing. There is no merging and no delta.
3. **The window keeps the copy warm.** A prefetch (`web/src/lib/offline.ts`) reads the lists and, for each
   course, its info, content tree, item facts, grades and published items, at start, when the
   connection returns and every 30 minutes. Four requests at a time.
4. **Writing.** A request may ask to be queued, and then carries an `Idempotency-Key` (ADR-0058) made by
   the window. It is sent at once if nothing is waiting. If the server cannot be reached, or answers `408`,
   `429` or `5xx`, it joins the queue with the same key. Only a student's draft and submit are queued
   (`PUT /assignments/{id}/submission/draft`, `POST .../submit`, for the document assignment and the
   form); a teacher's changes and a quiz need a connection and fail as they do online.
5. **Order and conflicts.** The queue is first in, first out. While anything waits, a new write joins the
   queue instead of passing it. A draft replaces the draft still waiting for the same path, which is last
   write wins within one assignment, unless a submit was queued after it (the submit means the older
   text). A row that is being sent is never replaced.
6. **Sending.** One loop sends what is due: a network error, `408`, `429` and `5xx` retry after 5, 10, 20,
   ... up to 300 seconds (`Retry-After` if longer); `401` holds the whole queue, losing nothing, until the
   person signs in again; any other answer is final. A final refusal (`409 "the submission window is
   closed"`, `422`, `403`) marks the row `failed` and keeps the server's problem text. The window shows
   it and the person discards it. It never retries by itself, and it does not hold up the rows behind it.
   The app never decides a deadline: it queues, the server answers.
7. **What the window sees.** The `sync` event and `sync_state` carry `online`, `pending`, `failed`,
   `paused`, `last_ok` and the paths that wait. A queued draft is shown as saved on this device; a
   queued submit says it waits to be sent, and the screen does not let the student edit meanwhile.
8. **Whose it is.** A different person signing in, and signing out, delete the cache and the queue. The
   app asks before signing out with writes waiting.
9. **At rest.** The file is not encrypted. It holds what the person could read in the app, in the user's
   own profile directory, as the browser cache of the web app would. Disk encryption of the device is the
   protection; the token is not in the file (it is in the OS keychain, ADR-0056).

## Consequences
- A deleted or unpublished course disappears from the app at the next successful read, not at once.
- A queued submit can be refused after the person has closed the window; the app shows the refusal the
  next time it is open, and the failed row stays until it is discarded.
- If the server was reached but the answer was lost, the retry meets the same key and gets the original
  answer (ADR-0058), so a submit is never made twice.
- The offline state is per device and per person. Nothing syncs between devices.
