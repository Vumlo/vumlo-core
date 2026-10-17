# 0043. Web interface language: a typed catalogue in the repository, no library

Status: Accepted
Supplements: ADR-0042

## Context
The institution has a default language and each person can pick one (Admin, Institution; Settings, This device). The web app showed English only, so the setting did nothing. The project owner asked for the setting to take effect. A new convention needs a record (AGENTS.md section 1).

## Options
- (a) An i18n library (`next-intl`, `react-i18next`): ICU messages, many features, a new dependency that ADR-0014's rule makes us propose, and `next-intl` assumes server rendering that ADR-0042 does not use.
- (b) **A small catalogue in `web/src/lib/i18n/`**: one TypeScript object per language, a `useT()` hook, and `Intl.PluralRules` for plurals. No dependency.
- (c) Translation files loaded at run time as JSON: extra requests, and keys are not checked at compile time.

## Decision
- Option (b). `en.ts` is the source: its keys are the type `MessageKey`. `uk.ts` must have every key or `pnpm typecheck` fails, so no language can fall behind silently. Values use `{name}` placeholders.
- A plural message has the keys `base.one` and `base.other`; Ukrainian adds `base.few` and `base.many`. Code calls `t.plural("base", n)`.
- Dates are built by `lib/format.ts` from per-language month and weekday tables, including the genitive month ("23 жовтня"). Counted nouns in `plural()` have Ukrainian forms there too.
- The language is the one stored in `localStorage` (`learnly.language`) on this device, otherwise the institution default, otherwise English. Changing it remounts the page tree, so text that plain functions produce follows.
- New text goes into the catalogue. Server text (problem details, mock to-do lines) is shown as the server sent it. Text in screens not yet moved to the catalogue stays English; moving a screen is a change that adds its keys to both files.

## Consequences
- The sign-in pages, the navbar and account menu, Courses, Home headings, Calendar controls, Settings, search, the new course page and the error pages are translated. The remaining screens (course content, items, editors, gradebook, admin) still show English text.
- No dependency is added.
