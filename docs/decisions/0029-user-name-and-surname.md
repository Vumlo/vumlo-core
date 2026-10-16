# 0029. A user has a name and a surname

Status: Accepted
Amends: ADR-0012, ADR-0017
Supplements: ADR-0003

## Context
`identity.users` holds one `display_name`. The gradebook (ADR-0028) lists students sorted by last name, then first name, and a teacher sees those names in it. One free-text field cannot be sorted that way, and splitting it at read time guesses wrong for every name that is not "first last".

## Options
- (a) Keep `display_name` and add `name` and `surname` beside it: two sources of truth that drift apart, and every client has to pick one.
- (b) Split `display_name` at read time: wrong for "van der Berg" and for names without a surname.
- (c) **Replace `display_name` with `name` and `surname`.** One place, sortable, and the password policy gets both words as context.

## Decision
- `identity.users` has `name` and `surname`, both `text NOT NULL`, trimmed, 1 to 100 characters, no control characters. `display_name` is dropped.
- The API replaces `display_name` with `name` and `surname` in `User`, `CreateUserRequest` and `UpdateUserRequest`. A missing or invalid part is a 422 on its own field. In `UpdateUserRequest` each part is optional and omitted means unchanged.
- The CLI takes `--name` and `--surname` in `create-admin` and `create-user`, both required.
- The migration splits existing rows at the first run of whitespace: the first word is the name, the rest is the surname. A one-word value becomes both parts.
- The password policy (ADR-0017) passes both words as context.
- This breaks the "additive only" rule of ADR-0012 once, like ADR-0024: nothing is published and no client exists yet. It is not repeated.

## Consequences
- The gradebook can sort by `(surname, name, id)`: "last name, first name" in ADR-0028 means `surname`, `name`.
- Names that are not "name surname" (a single name, several family names) fit with the surname holding whatever the person uses as such.
- A clean place for a full display string later is a computed value in the API, not a column.
