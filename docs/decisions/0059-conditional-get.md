# 0059. Conditional `GET`: `ETag` and `If-None-Match`

Status: Accepted
Amends: ADR-0012

## Context
The desktop app (ADR-0056, ADR-0060) keeps a copy of what it has read, and asks again when it is online.
Today every read transfers the whole answer. The task asked for `updated_since` or `ETag` on the read
endpoints the cache needs.

## Options
1. **`updated_since` per endpoint.** Rows are hard-deleted (no tombstones), `updated_at` does not move
   when a publish date passes or a roster changes through a group link, and `/courses/{id}/content` is
   one tree. Each endpoint would need its own definition of "changed", and several cannot give one.
2. **A generic `ETag`** of the body, with `If-None-Match`.
3. **Per-resource versions** in new columns. A migration of every table for a cache.

## Decision
1. **Option 2.** `server/src/conditional.rs` is a layer around the whole router. A successful `GET`
   under `/api/v1/` that answers `application/json` and says nothing about caching gets
   `ETag: W/"<first 128 bits of SHA-256 of the body>"` and `Cache-Control: private, no-cache`.
2. A request whose `If-None-Match` names the tag (a list, `W/` or not, or `*`) gets `304` with the `ETag`
   and no body.
3. Left alone: other methods, other statuses (errors never carry a tag), answers with `Cache-Control`,
   `Set-Cookie` or an `ETag` of their own.
4. The tag is a function of the body only. The body is a function of the person and the data, so the
   authorization of every request is decided as before and a tag presented by someone else is just a
   tag that does not match.

## Consequences
- The server does the same work as before; what is saved is the transfer. It buffers each answer in
  memory to hash it (they are built in memory already).
- `private, no-cache` makes a browser revalidate every `GET`, which is what a `304` answers cheaply.
  It does not forbid storing: the web app gets conditional requests from its HTTP cache for free.
- No tombstones: the offline store (ADR-0060) replaces what it holds on every successful `200` and
  keeps it on a `304`, so a deleted course disappears at the next read, not through a delta.
