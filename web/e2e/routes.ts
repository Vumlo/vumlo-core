import type { Page } from "@playwright/test";
import { courseHref, editItemHref, itemHref } from "../src/lib/routes";

/** Pages anyone can open without signing in. */
export const publicRoutes = ["/login", "/register", "/forgot-password", "/reset-password", "/verify-email", "/download"];

export type RoleName = "student" | "teacher" | "admin";

export const PASSWORD = "correct horse";

export const accounts: Record<RoleName, string> = {
  student: "maya.okafor@northfield.edu",
  teacher: "elena.voss@northfield.edu",
  admin: "marta.kovalska@northfield.edu",
};

/** Screens that do not depend on ids. */
export const staticRoutes: Record<RoleName, string[]> = {
  student: ["/", "/courses", "/calendar", "/grades", "/profile", "/search?q=merge", "/search", "/no-such-page"],
  teacher: ["/", "/courses", "/courses/new", "/calendar", "/grading", "/profile", "/search?q=merge", "/no-such-page"],
  admin: ["/admin", "/admin/people", "/admin/groups", "/admin/courses", "/admin/institution", "/admin/audit", "/courses", "/calendar", "/profile", "/search?q=merge", "/no-such-page"],
};

/**
 * Puts a value in localStorage before the scripts of a page run. The script runs in every frame, and the
 * sandboxed frame of a plugin has no storage (ADR-0066), so a failure there is the expected answer.
 */
export async function store(page: Page, key: string, value: string) {
  await page.addInitScript(
    (entry) => {
      try {
        localStorage.setItem(entry.key, entry.value);
      } catch {
        // A frame without storage.
      }
    },
    { key, value },
  );
}

type TreeItem = { id: string; kind: string };
type Tree = { modules: { items: TreeItem[]; submodules: { items: TreeItem[] }[] }[] };

async function api<T>(page: Page, path: string): Promise<T> {
  return page.evaluate(async (p) => (await fetch(`/api/v1${p}`)).json(), path);
}

/** Every course screen and every item of every course the signed-in person can open. */
export async function courseRoutes(page: Page, role: RoleName): Promise<string[]> {
  const courses = await api<{ items: { id: string }[] }>(page, "/me/courses?limit=100");
  const routes: string[] = [];
  for (const { id } of courses.items) {
    routes.push(courseHref(id), courseHref(id, "people"), courseHref(id, "grades"));
    if (role !== "student") routes.push(courseHref(id, "settings"));
    const tree = await api<Tree>(page, `/courses/${id}/content`);
    for (const m of tree.modules) {
      for (const it of [...m.items, ...m.submodules.flatMap((s) => s.items)]) {
        routes.push(itemHref(id, it.id));
        // The tab of the mock plugin "words" (ADR-0066) is on every assignment.
        if (it.kind === "assignment") routes.push(itemHref(id, it.id, "words"));
        if (role !== "student") routes.push(editItemHref(id, it.id));
      }
    }
  }
  return routes;
}
