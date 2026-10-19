# web/AGENTS.md

Rules for any coding agent working in `web/`. The root `AGENTS.md` and the ADRs still apply; ADR-0042 is the frontend decision.

## Stack and scripts
Next.js (App Router), React, TypeScript, CSS Modules. pnpm.

    pnpm dev:mock      # the app against the MSW mock API (no backend needed)
    pnpm dev           # the app proxied to the Rust server (VUMLO_API_ORIGIN)
    pnpm lint          # eslint + no colour literals outside styles/tokens.css
    pnpm typecheck     # needs `vumlo-cli brand apply` first (see below)
    pnpm build         # likewise
    pnpm build:desktop # the static export the desktop app loads (web/out, ADR-0057)
    pnpm test:e2e      # Playwright smoke test over every screen, both themes
    pnpm gen:api       # web/openapi.json -> src/lib/api/schema.d.ts

## Branding and optional modules (ADR-0054)
The web build takes its configuration from one place: `.vumlo/generated/` (next to `vumlo.toml`),
written by `vumlo-cli brand apply`. `scripts/brand.mjs` copies it into `public/brand/` (icons,
manifest, logo), `src/generated/` (`client-config.json` and its types) and `src/modules/enabled.ts`
(the UI of the optional modules that are on). All three are generated and gitignored. `typecheck` and
`build` stop with a clear message if the directory is missing; `dev` and `dev:mock` use a stand-in.

- Read the institution's public settings from `src/lib/clientConfig.ts`. Never read `vumlo.toml`, and
  never add a key to the web build that is not in `client-config.json`.
- The UI of a module lives in `src/modules/<name>/index.tsx` and nowhere else; it is rendered at
  `/m?name=<name>&path=...` and reaches the bundle only while the module is on.
- Theming is out of scope: universities edit `web/` themselves. This may conflict with upstream
  updates. A config change means `brand apply`, a web rebuild and a restart.

## Rules
- Ids are in the query string (`/course?id=...`), never in the path, and an address is built only by `src/lib/routes.ts` (ADR-0057). A page below `/course` reads them with `src/lib/useRoute.ts` and sits under the Suspense boundary of its layout.
- In the desktop app (ADR-0056, ADR-0060) every request goes through `lib/api/client.ts`, which hands it to the app; never call `fetch` for the API anywhere else. A write a student may make offline passes `{ queue: true }`; a screen that needs a connection (a quiz) says so with `useOffline()`.
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
