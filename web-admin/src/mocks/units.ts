import { http, HttpResponse } from "msw";
import { getDb } from "@/mocks/db";
import { problem, requireAdmin } from "@/mocks/logic";

const A = "/api/v1";
type Ability = "view_grades" | "edit_content" | "grade" | "manage_roster" | "manage_courses" | "manage_units";
type Unit = { id: string; name: string; parent_id: string | null; created_at: string };
type Role = "manager" | "viewer";

// The units of the mock: a faculty with courses and a department inside it, and another faculty. Elena Voss
// manages Computer Science. The abilities of the two roles are switched for every unit at once (ADR-0094).
const units: Unit[] = [
  { id: "unit-science", name: "Faculty of Science", parent_id: null, created_at: "2026-09-01T08:00:00Z" },
  { id: "unit-cs", name: "Computer Science", parent_id: "unit-science", created_at: "2026-09-01T08:05:00Z" },
  { id: "unit-arts", name: "Faculty of Arts", parent_id: null, created_at: "2026-09-01T08:10:00Z" },
];
const courses: Record<string, number> = { "unit-science": 2 };
const roles: Record<string, Record<string, Role>> = { "unit-cs": {} };
const abilities: Record<Role, Ability[]> = {
  manager: ["view_grades", "edit_content", "grade", "manage_roster", "manage_courses"],
  viewer: ["view_grades"],
};
const ALL: Ability[] = ["view_grades", "edit_content", "grade", "manage_roster", "manage_courses", "manage_units"];

const conflict = (detail: string) => problem(409, "Conflict", detail);
const unit = (id: string) => units.find((u) => u.id === id) ?? (() => { throw problem(404, "Not found"); })();
const taken = (name: string, parent: string | null, except?: string) =>
  units.some((u) => u.id !== except && u.parent_id === parent && u.name.toLowerCase() === name.toLowerCase());
const below = (id: string): boolean => units.some((u) => u.parent_id === id);
const inside = (id: string, ancestor: string): boolean => id === ancestor || (unit(id).parent_id !== null && inside(unit(id).parent_id!, ancestor));
const cleanName = (raw: unknown) => {
  const name = typeof raw === "string" ? raw.trim() : "";
  if (name.length < 1 || name.length > 100) throw problem(422, "Validation failed", undefined, [{ field: "name", message: "must be 1 to 100 characters" }]);
  return name;
};
const person = (id: string) => getDb().users.find((u) => u.id === id)!;

export const unitHandlers = [
  http.get(`${A}/units`, () => {
    requireAdmin();
    const items = [...units].sort((a, b) => a.name.localeCompare(b.name)).map((u) => ({ id: u.id, name: u.name, parent_id: u.parent_id, abilities: ALL }));
    return HttpResponse.json({ items, next_cursor: null });
  }),
  http.post(`${A}/units`, async ({ request }) => {
    requireAdmin();
    const b = (await request.json()) as { name?: string; parent_id?: string | null };
    const name = cleanName(b.name);
    const parent = b.parent_id ?? null;
    if (parent) unit(parent);
    if (taken(name, parent)) throw conflict("a unit with this name already exists here");
    const created: Unit = { id: `unit-${units.length + 1}-${name.toLowerCase().replace(/\W+/g, "-")}`, name, parent_id: parent, created_at: "2026-11-13T08:00:00Z" };
    units.push(created);
    return HttpResponse.json(created, { status: 201 });
  }),
  http.patch(`${A}/units/:id`, async ({ params, request }) => {
    requireAdmin();
    const u = unit(String(params.id));
    const b = (await request.json()) as { name?: string | null; parent_id?: string | null };
    const name = b.name != null ? cleanName(b.name) : u.name;
    const parent = "parent_id" in b ? (b.parent_id ?? null) : u.parent_id;
    if (parent && inside(parent, u.id)) throw problem(422, "Validation failed", undefined, [{ field: "parent_id", message: "a unit cannot move under itself or below" }]);
    if (taken(name, parent, u.id)) throw conflict("a unit with this name already exists here");
    u.name = name;
    u.parent_id = parent;
    return HttpResponse.json(u);
  }),
  http.delete(`${A}/units/:id`, ({ params }) => {
    requireAdmin();
    const u = unit(String(params.id));
    if (below(u.id) || (courses[u.id] ?? 0) > 0) throw conflict("the unit still has units or courses inside it");
    units.splice(units.indexOf(u), 1);
    delete roles[u.id];
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/units/:id/roles`, ({ params }) => {
    requireAdmin();
    const held = roles[unit(String(params.id)).id] ?? {};
    // Elena Voss manages Computer Science from the start.
    const items = Object.entries(held).map(([user_id, role]) => ({ user_id, role, name: person(user_id).name, surname: person(user_id).surname, email: person(user_id).email }));
    return HttpResponse.json({ items, next_cursor: null });
  }),
  http.put(`${A}/units/:id/roles/:userId`, async ({ params, request }) => {
    requireAdmin();
    const id = unit(String(params.id)).id;
    const b = (await request.json()) as { role: Role };
    (roles[id] ??= {})[String(params.userId)] = b.role;
    return new HttpResponse(null, { status: 204 });
  }),
  http.delete(`${A}/units/:id/roles/:userId`, ({ params }) => {
    requireAdmin();
    const held = roles[unit(String(params.id)).id] ?? {};
    delete held[String(params.userId)];
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/units/:id/candidates`, ({ params, request }) => {
    requireAdmin();
    const held = roles[unit(String(params.id)).id] ?? {};
    const q = (new URL(request.url).searchParams.get("q") ?? "").trim().toLowerCase();
    const items = getDb().users
      .filter((u) => u.status === "active" && (u.role === "teacher" || u.role === "admin") && !(u.id in held))
      .filter((u) => q.length > 0 && `${u.name} ${u.surname} ${u.email}`.toLowerCase().includes(q))
      .sort((a, b) => a.surname.localeCompare(b.surname))
      .slice(0, 20)
      .map((u) => ({ user_id: u.id, name: u.name, surname: u.surname, email: u.email, role: u.role }));
    return HttpResponse.json({ items });
  }),
  http.get(`${A}/admin/unit-roles`, () => {
    requireAdmin();
    return HttpResponse.json(abilities);
  }),
  http.put(`${A}/admin/unit-roles/:role`, async ({ params, request }) => {
    requireAdmin();
    const role = params.role as Role;
    if (role !== "manager" && role !== "viewer") throw problem(404, "Not found");
    abilities[role] = ((await request.json()) as { abilities: Ability[] }).abilities;
    return HttpResponse.json(abilities);
  }),
];

/** Elena Voss manages Computer Science: the mock's first role, set once the seed of people exists. */
export function seedUnitRoles() {
  const elena = getDb().users.find((u) => u.role === "teacher" && u.surname === "Voss");
  if (elena) (roles["unit-cs"] ??= {})[elena.id] = "manager";
}
