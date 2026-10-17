# web/AGENTS.md

Rules for any coding agent working in `web/`. The root `AGENTS.md` and the ADRs still apply; ADR-0042 is the frontend decision.

## Stack and scripts
Next.js (App Router), React, TypeScript, CSS Modules. pnpm.

    pnpm dev:mock      # the app against the MSW mock API (no backend needed)
    pnpm dev           # the app proxied to the Rust server (LEARNLY_API_ORIGIN)
    pnpm lint          # eslint + no colour literals outside styles/tokens.css
    pnpm typecheck
    pnpm test:e2e      # Playwright smoke test over every screen, both themes
    pnpm gen:api       # web/openapi.json -> src/lib/api/schema.d.ts

## Rules
- Every page is a client component. The browser calls `/api/v1` only; no route handlers, no server actions.
- Data comes from typed hooks in `src/lib/api/`. A hook for an endpoint the server has uses the generated types. A hook for a missing endpoint or field uses the hand-written types in `src/lib/api/gaps.ts`, is mocked in `src/mocks/`, and has an entry in `API_GAPS.md` (screen, what is missing, what is mocked). Never change the backend from a UI task.
- Colours only as `var(--token)` from `styles/tokens.css`. No hex, `rgb()` or `hsl()` anywhere else (`pnpm lint` checks).
- Points are decimal strings (ADR-0021). Use `lib/points.ts`; never `parseFloat` or `Number` on a score.
- Lectures and assignments always render with `ItemTile` (own colour and icon).
- Roles hide actions only. Never rely on the UI for permission; show what the server returns and surface its 403/404.
- Student views never show `draft_*` grade fields (root AGENTS.md section 6). Only `published_*` reach student components.
- A new dependency, or a version bump, needs an ADR first.
- Dates come from `lib/clock.ts` `now()`, so the mock runs on a fixed clock.
- Every screen opens under `pnpm dev:mock` in light and dark with no console errors; add new routes to `e2e/routes.ts`.
