# 0057. Query-string routes and the desktop export build

Status: Accepted
Amends: ADR-0042, ADR-0054 (the address of a module page), ADR-0056

## Context
The desktop app (ADR-0056) loads a static export of `web/`. An export is built before any user exists,
so every page of it must be listed at build time. The course screens were `courses/[courseId]/...` and
`items/[itemId]`, whose ids are runtime UUIDs, and `m/[module]/[[...path]]` and `[...rest]` were
dynamic too. A desktop webview has no server to rewrite an unbuilt path to a shell page, and the spike
(ADR-0056) showed Tauri answers an unknown path with the home page.

## Options
1. **Keep path ids, pre-render one placeholder** (`/courses/_`) and rewrite everything else to it. The
   web server can do that; the desktop cannot, so the two builds would differ in their routing.
2. **A separate SPA for the desktop** with its own router and shared packages. Two applications.
3. **Ids in the query string**, one scheme for both builds.

## Decision
1. **Option 3.** The routes are
   - `/course?id=<course>` and `/course/{grades,people,settings}?id=<course>`;
   - `/course/item?id=<course>&item=<item>` and `/course/item/edit?id=<course>&item=<item>`;
   - `/m?name=<module>&path=a/b`.

   `/courses`, `/courses/new` and every other screen keep their paths. Addresses are built only by
   `src/lib/routes.ts` and read only by the hooks of `src/lib/useRoute.ts`; a screen below `/course`
   can rely on `useCourseId()` because `course/layout.tsx` shows "not found" without a course.
2. **Old addresses are not redirected.** `/courses/<id>/...` was never sent by the server (mail links
   are for sign-in, verification and password reset), and a redirect table would be a compatibility
   layer (AGENTS §13). A bookmark to an old address shows "not found" and the person opens the course
   from the list.
3. **The unknown address.** `(app)/[...rest]` is deleted. `app/not-found.tsx` renders the "not found"
   screen inside `Shell`; the web server answers such an address with a real `404`. A static export
   has no use for it (Tauri serves the home page), so the desktop app never builds an unknown path.
4. **Two builds from one source.**
   - `npm run build`: as before, a Next.js server (`next start`), with the rewrite of `/api/v1` and the
     cookie session. The web app keeps its relative `/api/v1` address, because the session cookie needs
     the same site (ADR-0013). The address of the API is configured where it is needed: in the desktop
     build, from `client-config.json` (ADR-0056).
   - `npm run build:desktop` (`scripts/build-desktop.mjs`): `VUMLO_TARGET=desktop`, `output: "export"`,
     no rewrites, into `web/out`, without `mockServiceWorker.js`.
5. **Transport.** `lib/api/client.ts` chooses at run time: `fetch` in a browser, `invoke("api")`
   (`@tauri-apps/api`) when `__TAURI_INTERNALS__` exists. Login sends `client: "app"` on the desktop.
   The error mapping (`ApiError`, status 0 for no connection) is the same for both.
6. **No mock server in a build.** `next.config.ts` aliases `@/mocks/StartWorker` to an empty module
   unless `NEXT_PUBLIC_API_MOCKING=enabled`, because a dead `import()` still bundles msw.
7. **CSP** is set for the desktop only, in `tauri.conf.json`. Tauri hashes the inline scripts of the
   exported HTML. The web app gets none: Next.js's inline scripts would need a per-request nonce, which
   means dynamic rendering, against ADR-0042; the reverse proxy can add headers that do not depend on it.
8. **A student's draft is sent when the page is left.** `AssignmentPage` and `SheetPage` flushed
   nothing on unmount and lost the last second of typing; they now send the pending save.

## Consequences
- Every link to a course or item changes; the 49 builders now call `routes.ts`, and `e2e/routes.ts`
  uses the same functions.
- The unknown-address smoke test now asserts the `404` status and ignores the browser's own console
  line about it.
- `/courses/<id>` bookmarks are lost once (see decision 2).
- `next.config.ts` must stay free of `headers()`, `redirects()` and anything else an export rejects when
  `VUMLO_TARGET=desktop`.
