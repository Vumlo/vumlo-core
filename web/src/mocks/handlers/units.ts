import { http, HttpResponse } from "msw";
import { getDb, type DbUnit, type DbUser } from "../db";
import { canView, courseById, forbidden, grantOn, isConsoleAdmin, isStaff, notFound, paged, problem, requireAdmin, requireUser, unitById, unitInside } from "../logic";
import type { UnitAbility, UnitRole } from "@/lib/api/hooks/units";
import { publicCourse } from "./core";

const A = "/api/v1";

// The units are seeded in `seed.ts`. Who sees what follows the server (ADR-0094, ADR-0095): in the console an admin holds
// every ability on every unit, in the people's app everybody sees the units their roles cover. The two roles' abilities are
// switched for every unit at once.
const conflict = (detail: string) => problem(409, "Conflict", detail);
const taken = (name: string, parent: string | null, except?: string) =>
  getDb().units.some((u) => u.id !== except && u.parent_id === parent && u.name.toLowerCase() === name.toLowerCase());
const cleanName = (raw: unknown) => {
  const name = typeof raw === "string" ? raw.trim() : "";
  if (name.length < 1 || name.length > 100) throw problem(422, "Validation failed", undefined, [{ field: "name", message: "must be 1 to 100 characters" }]);
  return name;
};
const person = (id: string) => getDb().users.find((u) => u.id === id)!;
/** The abilities on a unit for a caller who sees it, or a 404 for one who does not. */
const seen = (user: DbUser, unitId: string) => {
  const grant = grantOn(user, unitById(unitId).id);
  if (!grant) throw notFound();
  return grant;
};
/** The unit, for a caller who sees it. */
const visible = (user: DbUser, unitId: string) => {
  seen(user, unitId);
  return unitById(unitId);
};
/** 403 without `manage_units` on `parent`; a top-level unit (no parent) is the console's alone. */
const mustManageIn = (user: DbUser, parent: string | null) => {
  if (!(parent === null ? isConsoleAdmin(user) : grantOn(user, parent)?.has("manage_units"))) throw forbidden();
};
/** A parent named by a request body: one that does not exist or the caller does not see is a bad field. */
const parentField = (user: DbUser, id: string) => {
  if (!getDb().units.some((u) => u.id === id) || !grantOn(user, id)) throw problem(422, "Validation failed", undefined, [{ field: "parent_id", message: "there is no such unit" }]);
};
/** A unit's roles are changed from above: `manage_units` on its parent (spec 5; nobody changes the roles of the unit their own role sits on). */
const managedFromAbove = (user: DbUser, unitId: string) => mustManageIn(user, visible(user, unitId).parent_id);

export const unitHandlers = [
  http.get(`${A}/units`, () => {
    const user = requireUser();
    const items = getDb().units
      .map((u) => ({ id: u.id, name: u.name, parent_id: u.parent_id, grant: grantOn(user, u.id) }))
      .filter((u) => u.grant)
      .sort((a, b) => a.name.localeCompare(b.name))
      .map(({ grant, ...u }) => ({ ...u, abilities: [...grant!] }));
    return HttpResponse.json({ items, next_cursor: null });
  }),
  http.get(`${A}/units/:id/courses`, ({ params, request }) => {
    const id = String(params.id);
    seen(requireUser(), id);
    const items = getDb().courses.filter((c) => c.unit_id && unitInside(c.unit_id, id)).sort((a, b) => a.id.localeCompare(b.id)).map(publicCourse);
    return paged(items, request);
  }),
  // Needs `manage_courses` on the unit the course leaves and on the one it enters (ADR-0094); "no unit" needs only the first.
  http.put(`${A}/courses/:id/unit`, async ({ params, request }) => {
    const user = requireUser();
    const course = courseById(String(params.id));
    if (!canView(user, course)) throw notFound();
    if (!isStaff(user, course)) throw forbidden();
    const b = (await request.json()) as { unit_id?: string | null };
    if (b.unit_id === undefined) throw problem(422, "Validation failed", undefined, [{ field: "unit_id", message: "is required: a unit, or null for none" }]);
    const manages = (unitId: string) => grantOn(user, unitId)?.has("manage_courses");
    if (course.unit_id && !manages(course.unit_id)) throw forbidden();
    if (b.unit_id !== null) {
      if (!getDb().units.some((u) => u.id === b.unit_id)) throw problem(422, "Validation failed", undefined, [{ field: "unit_id", message: "there is no such unit" }]);
      if (!manages(b.unit_id)) throw forbidden();
    }
    course.unit_id = b.unit_id;
    return HttpResponse.json(publicCourse(course));
  }),
  http.post(`${A}/units`, async ({ request }) => {
    const user = requireUser();
    const { units } = getDb();
    const b = (await request.json()) as { name?: string; parent_id?: string | null };
    const name = cleanName(b.name);
    const parent = b.parent_id ?? null;
    if (parent) parentField(user, parent);
    mustManageIn(user, parent);
    if (taken(name, parent)) throw conflict("a unit with this name already exists here");
    const created: DbUnit = { id: `unit-${units.length + 1}-${name.toLowerCase().replace(/\W+/g, "-")}`, name, parent_id: parent, created_at: "2026-11-13T08:00:00Z" };
    units.push(created);
    return HttpResponse.json(created, { status: 201 });
  }),
  http.patch(`${A}/units/:id`, async ({ params, request }) => {
    const user = requireUser();
    const u = visible(user, String(params.id));
    mustManageIn(user, u.parent_id);
    const b = (await request.json()) as { name?: string | null; parent_id?: string | null };
    const name = b.name != null ? cleanName(b.name) : u.name;
    const parent = "parent_id" in b ? (b.parent_id ?? null) : u.parent_id;
    if (parent !== u.parent_id) {
      if (parent) {
        parentField(user, parent);
        if (unitInside(parent, u.id)) throw problem(422, "Validation failed", undefined, [{ field: "parent_id", message: "a unit cannot move under itself or below" }]);
      }
      mustManageIn(user, parent);
    }
    if (taken(name, parent, u.id)) throw conflict("a unit with this name already exists here");
    u.name = name;
    u.parent_id = parent;
    return HttpResponse.json(u);
  }),
  http.delete(`${A}/units/:id`, ({ params }) => {
    const user = requireUser();
    const { units, courses, unitRoles } = getDb();
    const u = visible(user, String(params.id));
    mustManageIn(user, u.parent_id);
    if (units.some((x) => x.parent_id === u.id) || courses.some((c) => c.unit_id === u.id)) throw conflict("the unit still has units or courses inside it");
    units.splice(units.indexOf(u), 1);
    delete unitRoles[u.id];
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/units/:id/roles`, ({ params }) => {
    const held = getDb().unitRoles[visible(requireUser(), String(params.id)).id] ?? {};
    const items = Object.entries(held).map(([user_id, role]) => ({ user_id, role, name: person(user_id).name, surname: person(user_id).surname, email: person(user_id).email }));
    return HttpResponse.json({ items, next_cursor: null });
  }),
  http.put(`${A}/units/:id/roles/:userId`, async ({ params, request }) => {
    managedFromAbove(requireUser(), String(params.id));
    const id = String(params.id);
    const b = (await request.json()) as { role: UnitRole };
    (getDb().unitRoles[id] ??= {})[String(params.userId)] = b.role;
    return new HttpResponse(null, { status: 204 });
  }),
  http.delete(`${A}/units/:id/roles/:userId`, ({ params }) => {
    managedFromAbove(requireUser(), String(params.id));
    const held = getDb().unitRoles[String(params.id)] ?? {};
    delete held[String(params.userId)];
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/units/:id/candidates`, ({ params, request }) => {
    managedFromAbove(requireUser(), String(params.id));
    const held = getDb().unitRoles[String(params.id)] ?? {};
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
    return HttpResponse.json(getDb().unitAbilities);
  }),
  http.put(`${A}/admin/unit-roles/:role`, async ({ params, request }) => {
    requireAdmin();
    const role = params.role as UnitRole;
    if (role !== "manager" && role !== "viewer") throw problem(404, "Not found");
    getDb().unitAbilities[role] = ((await request.json()) as { abilities: UnitAbility[] }).abilities;
    return HttpResponse.json(getDb().unitAbilities);
  }),
];
