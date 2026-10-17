import type { Page } from "@playwright/test";

export type RoleName = "student" | "teacher" | "admin";

export const PASSWORD = "correct horse";

export const accounts: Record<RoleName, string> = {
  student: "maya.okafor@northfield.edu",
  teacher: "elena.voss@northfield.edu",
  admin: "marta.kovalska@northfield.edu",
};

/** Screens that do not depend on ids. */
export const staticRoutes: Record<RoleName, string[]> = {
  student: ["/", "/courses", "/calendar", "/grades", "/profile"],
  teacher: ["/", "/courses", "/calendar", "/grading", "/profile"],
  admin: ["/admin", "/admin/people", "/admin/groups", "/admin/courses", "/admin/institution", "/admin/audit", "/courses", "/calendar", "/profile"],
};

type Tree = { modules: { items: { id: string }[]; submodules: { items: { id: string }[] }[] }[] };

async function api<T>(page: Page, path: string): Promise<T> {
  return page.evaluate(async (p) => (await fetch(`/api/v1${p}`)).json(), path);
}

/** Every course screen and every item of every course the signed-in person can open. */
export async function courseRoutes(page: Page, role: RoleName): Promise<string[]> {
  const courses = await api<{ items: { id: string }[] }>(page, "/me/courses?limit=100");
  const routes: string[] = [];
  for (const { id } of courses.items) {
    routes.push(`/courses/${id}`, `/courses/${id}/people`);
    if (role !== "student") routes.push(`/courses/${id}/grades`, `/courses/${id}/settings`);
    else routes.push(`/courses/${id}/grades`);
    const tree = await api<Tree>(page, `/courses/${id}/content`);
    for (const m of tree.modules) {
      for (const it of [...m.items, ...m.submodules.flatMap((s) => s.items)]) {
        routes.push(`/courses/${id}/items/${it.id}`);
        if (role !== "student") routes.push(`/courses/${id}/items/${it.id}/edit`);
      }
    }
  }
  return routes;
}
