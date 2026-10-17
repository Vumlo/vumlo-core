# 0042. Web frontend: Next.js, client-rendered, typed hooks over a generated API schema

Status: Accepted
Supplements: ADR-0012, ADR-0013

## Context
ADR-0013 left the frontend technology open ("React SPA or Leptos"). The `web/` application is now built from the design canvas, so the choice has to be recorded together with the dependencies it brings. The project owner chose Next.js.

## Options
Rendering: (a) server components that call the API on the server — the session cookie would need forwarding and every screen would need a server-side data path next to the browser one; (b) **client components only**, the browser calls `/api/v1` like any other client (ADR-0012, ADR-0013). Next.js then provides routing, bundling and `next/font`, and nothing else.

Data layer: (a) TanStack Query — a good fit, but one more dependency for what is a cache keyed by URL with invalidation; (b) **a small `useQuery`/`useMutation` pair in `web/src/lib/query.ts`** (about 100 lines).

Styling: (a) Tailwind — a build plugin and a second place for colours; (b) **CSS Modules plus a few global classes**, colours only as CSS variables from `styles/tokens.css`.

Types: (a) hand-written DTOs — drift from the server; (b) **`openapi-typescript` generates `src/lib/api/schema.d.ts` from `web/openapi.json`**, which is the spec the server publishes at `/api/v1/openapi.json`.

## Decision
- `web/` is a Next.js App Router application. Every page is a client component. There are no route handlers and no server actions; the browser calls the public API only.
- `next.config.ts` rewrites `/api/v1/*` to the Rust server in development (`LEARNLY_API_ORIGIN`). In production a reverse proxy serves both on one site (ADR-0013), so the `learnly_session` cookie is same-origin and the CSRF check of ADR-0003 passes.
- Dependencies (pinned in `web/package.json`, installed with pnpm):
  - runtime: `next`, `react`, `react-dom`;
  - development: `typescript`, `eslint`, `eslint-config-next`, `@types/*`, `openapi-typescript` (schema types), `msw` (the mock API for `pnpm dev:mock` and the smoke test), `@playwright/test` (the smoke test).
- Fonts come from `next/font/google`, which downloads them at build time and serves them from our own origin. Nothing loads from a font CDN at run time.
- Colours: the light and dark tokens from the design canvas live in `styles/tokens.css` as CSS variables, switched by `data-theme` on `<html>`. No other file contains a colour literal; `pnpm lint` fails if one does.
- Points are decimal strings (ADR-0021). The UI never converts a score to a float; `lib/points.ts` does arithmetic on integer hundredths.
- A screen reads the API only through a typed hook in `src/lib/api/`. When the server has no endpoint for a screen, or lacks a field, the hook still exists, its types are written by hand in `src/lib/api/gaps.ts`, the response is mocked by MSW, and the gap is recorded in `web/API_GAPS.md`. The backend is not changed by a UI task.
- Roles in the UI only hide actions. The server enforces every permission (ADR-0004).
- Block documents (ADR-0025, ADR-0027) are rendered by the client from the stored JSON. Formulas and Mermaid diagrams are shown as their source text until a renderer is accepted by its own ADR; each would be a new dependency.

## Consequences
- One toolchain (Node 22+, pnpm 10) next to the Rust workspace; `web/` has its own lockfile and is not part of `cargo` checks.
- The hand-written `gaps.ts` types are the list of work for the API tasks that follow; deleting an entry there is the sign that a gap is closed.
- Server-side rendering is not used, so the first paint shows a loading state. Revisit if an institution needs SEO for public pages.
- KaTeX and Mermaid rendering, and syntax highlighting beyond the small built-in tokenizer, each need their own ADR.
