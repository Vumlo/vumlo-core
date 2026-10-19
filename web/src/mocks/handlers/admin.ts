import { http, HttpResponse } from "msw";
import { MOCK_NOW } from "@/lib/clock";
import type { AdminCourse, AppPlatform, GroupOverview, TermStat, UserFacts } from "@/lib/api/types";
import { auditRow, getDb, uid } from "../db";
import { activeStudents, fullName, notFound, paged, problem, publicUser, requireAdmin, requireUser, teacherName, userById } from "../logic";

const A = "/api/v1";

const TEMP = ["amber", "river", "cedar", "linen", "harbor", "violet", "ember", "lantern"];

function tempPassword() {
  const db = getDb();
  return `${TEMP[db.audit.length % TEMP.length]}-${TEMP[(db.audit.length * 3 + 1) % TEMP.length]}-${4000 + db.audit.length * 7}`;
}

function record(actor: { id: string; name: string; surname: string }, action: string, resource: { type: string; id: string; label: string | null }, meta: Record<string, unknown> = {}) {
  const db = getDb();
  db.audit.unshift(auditRow(`${db.audit.length}:${action}`, MOCK_NOW.toISOString(), actor, action, resource, meta));
}

export const adminHandlers = [
  http.get(`${A}/admin/users`, ({ request }) => {
    requireAdmin();
    const q = new URL(request.url).searchParams;
    let list = getDb().users.slice();
    const role = q.get("role");
    const status = q.get("status");
    const email = q.get("email")?.toLowerCase();
    if (role) list = list.filter((u) => u.role === role);
    if (status) list = list.filter((u) => u.status === status);
    if (email) list = list.filter((u) => u.email.includes(email) || fullName(u).toLowerCase().includes(email));
    return paged(list.sort((a, b) => a.created_at.localeCompare(b.created_at)).map(publicUser), request);
  }),
  http.get(`${A}/admin/users/summary`, () => {
    requireAdmin();
    const u = getDb().users;
    return HttpResponse.json({
      all: u.length, student: u.filter((x) => x.role === "student").length, teacher: u.filter((x) => x.role === "teacher").length,
      admin: u.filter((x) => x.role === "admin").length, disabled: u.filter((x) => x.status === "disabled").length,
    });
  }),
  http.get(`${A}/admin/users/:id/facts`, ({ params }) => {
    requireAdmin();
    const db = getDb();
    const u = userById(String(params.id));
    const line = (c: { id: string; code: string; title: string }) => ({ course_id: c.id, code: c.code, title: c.title });
    const regular = (id: string) => db.courses.find((c) => c.id === id && c.kind === "regular");
    const facts: UserFacts = {
      role: u.role,
      status: u.status,
      created_at: u.created_at,
      last_login_at: u.last_sign_in,
      groups: db.groupMembers.filter((g) => g.user_id === u.id).map((g) => ({ id: g.group_id, name: db.groups.find((x) => x.id === g.group_id)!.name })),
      owns: db.courses.filter((c) => c.owner_id === u.id && c.kind === "regular").map(line),
      teaches: db.members.filter((m) => m.user_id === u.id && m.role !== "student" && !m.is_owner).flatMap((m) => regular(m.course_id) ?? []).map(line),
      studies: db.members.filter((m) => m.user_id === u.id && m.role === "student" && !m.left_on && regular(m.course_id)?.status === "published").length,
    };
    return HttpResponse.json(facts);
  }),
  http.post(`${A}/admin/users`, async ({ request }) => {
    const admin = requireAdmin();
    const b = (await request.json()) as { email?: string; name?: string; surname?: string; role?: "admin" | "teacher" | "student" };
    const errors: { field: string; message: string }[] = [];
    if (!b.name?.trim()) errors.push({ field: "name", message: "Enter a first name" });
    if (!b.surname?.trim()) errors.push({ field: "surname", message: "Enter a surname" });
    if (!b.email?.includes("@")) errors.push({ field: "email", message: "Enter an email address" });
    if (errors.length) throw problem(422, "Validation failed", undefined, errors);
    const db = getDb();
    const email = b.email!.trim().toLowerCase();
    if (db.users.some((u) => u.email === email)) throw problem(409, "Conflict", "There is already an account with this email");
    const temporary_password = tempPassword();
    const user = {
      id: uid(`user:new:${db.users.length}:${email}`), email, name: b.name!.trim(), surname: b.surname!.trim(), role: b.role ?? "student", status: "active" as const,
      must_change_password: true, created_at: MOCK_NOW.toISOString(), password: temporary_password, last_sign_in: null,
    };
    db.users.push(user);
    record(admin, "user.create", { type: "user", id: user.id, label: fullName(user) }, { role: user.role });
    return HttpResponse.json({ user: publicUser(user), temporary_password }, { status: 201, headers: { "Cache-Control": "no-store" } });
  }),
  http.patch(`${A}/admin/users/:id`, async ({ request, params }) => {
    const admin = requireAdmin();
    const u = userById(String(params.id));
    const b = (await request.json()) as { name?: string; surname?: string; role?: "admin" | "teacher" | "student"; status?: "active" | "disabled" };
    const db = getDb();
    if (b.role && b.role !== u.role) {
      if (u.role === "teacher" && b.role === "student" && db.courses.some((c) => c.owner_id === u.id && c.kind === "regular")) {
        throw problem(409, "Conflict", "This teacher owns courses. Give them to another teacher first.");
      }
      record(admin, "user.role_change", { type: "user", id: u.id, label: fullName(u) }, { from: u.role, to: b.role });
      u.role = b.role;
    }
    if (b.status && b.status !== u.status) {
      record(admin, b.status === "disabled" ? "user.disable" : "user.enable", { type: "user", id: u.id, label: fullName(u) });
      u.status = b.status;
    }
    if (b.name?.trim()) u.name = b.name.trim();
    if (b.surname?.trim()) u.surname = b.surname.trim();
    return HttpResponse.json(publicUser(u));
  }),
  http.post(`${A}/admin/users/:id/reset-password`, ({ params }) => {
    const admin = requireAdmin();
    const u = userById(String(params.id));
    u.password = tempPassword();
    u.must_change_password = true;
    record(admin, "user.password_reset", { type: "user", id: u.id, label: fullName(u) });
    return HttpResponse.json({ temporary_password: u.password }, { headers: { "Cache-Control": "no-store" } });
  }),

  // ---- groups ----
  http.get(`${A}/groups`, ({ request }) => {
    requireAdmin();
    return paged(getDb().groups, request);
  }),
  http.post(`${A}/groups`, async ({ request }) => {
    const admin = requireAdmin();
    const b = (await request.json()) as { name?: string };
    const name = (b.name ?? "").trim();
    if (!name) throw problem(422, "Validation failed", undefined, [{ field: "name", message: "Give the group a name" }]);
    const db = getDb();
    if (db.groups.some((g) => g.name === name)) throw problem(409, "Conflict", "There is already a group with this name");
    const g = { id: uid(`group:new:${db.groups.length}:${name}`), name, created_at: MOCK_NOW.toISOString() };
    db.groups.push(g);
    record(admin, "group.create", { type: "group", id: g.id, label: name });
    return HttpResponse.json(g, { status: 201 });
  }),
  http.get(`${A}/groups/:id/subgroups`, ({ request, params }) => {
    requireAdmin();
    return paged(getDb().subgroups.filter((s) => s.group_id === params.id), request);
  }),
  http.get(`${A}/groups/:id/members`, ({ request, params }) => {
    requireAdmin();
    const rows = getDb().groupMembers.filter((g) => g.group_id === params.id).map((g) => userById(g.user_id)).sort((a, b) => a.surname.localeCompare(b.surname));
    return paged(rows.map((u) => ({ user_id: u.id, name: u.name, surname: u.surname })), request);
  }),
  http.get(`${A}/admin/groups/:id/overview`, ({ params }) => {
    requireAdmin();
    const db = getDb();
    const members = db.groupMembers.filter((g) => g.group_id === params.id).map((g) => ({ g, u: userById(g.user_id) })).sort((a, b) => a.u.surname.localeCompare(b.u.surname));
    const links = db.links.filter((l) => l.group_id === params.id);
    const overview: GroupOverview = {
      group_id: String(params.id), student_count: members.length, course_count: new Set(links.map((l) => l.course_id)).size,
      members: members.map(({ g, u }) => ({ user_id: u.id, name: u.name, surname: u.surname, email: u.email, added_at: g.since })),
      linked_courses: [...new Set(links.map((l) => l.course_id))].map((cid) => {
        const c = db.courses.find((x) => x.id === cid)!;
        const l = links.find((x) => x.course_id === cid)!;
        return { course_id: cid, code: c.code, title: c.title, owner_name: userById(c.owner_id).name, owner_surname: userById(c.owner_id).surname, linked_on: l.linked_on };
      }),
    };
    return HttpResponse.json(overview);
  }),
  http.post(`${A}/groups/:id/members`, async ({ request, params }) => {
    const admin = requireAdmin();
    const b = (await request.json()) as { user_ids: string[] };
    const db = getDb();
    const group = db.groups.find((g) => g.id === params.id);
    if (!group) throw notFound();
    let added = 0;
    for (const id of b.user_ids) {
      if (db.groupMembers.some((g) => g.group_id === group.id && g.user_id === id)) continue;
      db.groupMembers.push({ group_id: group.id, subgroup_id: null, user_id: id, since: MOCK_NOW.toISOString() });
      added++;
    }
    if (added) record(admin, "group.member.add", { type: "group", id: group.id, label: group.name });
    return HttpResponse.json({ added });
  }),
  http.post(`${A}/groups/:id/members/remove`, async ({ request, params }) => {
    const admin = requireAdmin();
    const b = (await request.json()) as { user_ids: string[] };
    const db = getDb();
    const group = db.groups.find((g) => g.id === params.id);
    if (!group) throw notFound();
    const before = db.groupMembers.length;
    db.groupMembers = db.groupMembers.filter((g) => !(g.group_id === group.id && b.user_ids.includes(g.user_id)));
    const removed = before - db.groupMembers.length;
    // A student who left a linked group leaves its courses; the grades stay and are locked.
    for (const m of db.members.filter((x) => x.source === "group" && b.user_ids.includes(x.user_id) && db.links.some((l) => l.course_id === x.course_id && l.group_id === group.id))) m.left_on = MOCK_NOW.toISOString();
    if (removed) record(admin, "group.members.remove", { type: "group", id: group.id, label: group.name });
    return HttpResponse.json({ removed });
  }),
  http.put(`${A}/groups/:id/members`, async ({ request, params }) => {
    const admin = requireAdmin();
    const b = (await request.json()) as { user_ids: string[] };
    const db = getDb();
    const group = db.groups.find((g) => g.id === params.id);
    if (!group) throw notFound();
    const current = new Set(db.groupMembers.filter((g) => g.group_id === group.id).map((g) => g.user_id));
    const next = new Set(b.user_ids);
    const removed = [...current].filter((x) => !next.has(x));
    const added = [...next].filter((x) => !current.has(x));
    db.groupMembers = db.groupMembers.filter((g) => !(g.group_id === group.id && removed.includes(g.user_id)));
    for (const id of added) db.groupMembers.push({ group_id: group.id, subgroup_id: null, user_id: id, since: MOCK_NOW.toISOString() });
    record(admin, "group.members.replace", { type: "group", id: group.id, label: group.name });
    return HttpResponse.json({ added: added.length, removed: removed.length });
  }),

  // ---- terms and courses ----
  http.get(`${A}/terms`, ({ request }) => {
    requireUser();
    return paged(getDb().terms, request);
  }),
  http.post(`${A}/terms`, async ({ request }) => {
    requireAdmin();
    const b = (await request.json()) as { name?: string; starts_on?: string; ends_on?: string };
    const errors: { field: string; message: string }[] = [];
    if (!b.name?.trim()) errors.push({ field: "name", message: "Give the term a name" });
    if (!b.starts_on) errors.push({ field: "starts_on", message: "Pick the first day" });
    if (!b.ends_on) errors.push({ field: "ends_on", message: "Pick the last day" });
    if (b.starts_on && b.ends_on && b.ends_on <= b.starts_on) errors.push({ field: "ends_on", message: "The last day must be after the first" });
    if (errors.length) throw problem(422, "Validation failed", undefined, errors);
    const db = getDb();
    if (db.terms.some((t) => t.name === b.name!.trim())) throw problem(409, "Conflict", "There is already a term with this name");
    const t = { id: uid(`term:new:${db.terms.length}:${b.name}`), name: b.name!.trim(), starts_on: b.starts_on!, ends_on: b.ends_on! };
    db.terms.push(t);
    return HttpResponse.json(t, { status: 201 });
  }),
  http.get(`${A}/admin/term-stats`, () => {
    requireAdmin();
    const db = getDb();
    const today = MOCK_NOW.toISOString().slice(0, 10);
    const items: TermStat[] = db.terms.map((t) => ({
      term_id: t.id, courses: db.courses.filter((c) => c.term_id === t.id && c.kind === "regular").length, read_only: t.ends_on < today, current: t.starts_on <= today && today <= t.ends_on,
    }));
    return HttpResponse.json({ items });
  }),
  http.get(`${A}/admin/courses`, () => {
    requireAdmin();
    const db = getDb();
    const items: AdminCourse[] = db.courses.filter((c) => c.kind === "regular").map((c) => ({
      course_id: c.id, code: c.code, title: c.title, owner_id: c.owner_id, owner_name: userById(c.owner_id).name, owner_surname: userById(c.owner_id).surname, term_id: c.term_id ?? null, term_name: db.terms.find((t) => t.id === c.term_id)?.name ?? null,
      student_count: activeStudents(c.id).length, status: c.status,
    }));
    return HttpResponse.json({ items });
  }),

  // ---- institution and audit ----
  http.get(`${A}/admin/institution`, () => {
    requireAdmin();
    return HttpResponse.json(getDb().institution);
  }),
  // The links to the desktop app: the whole list at once, https only, one per platform (ADR-0064).
  http.put(`${A}/admin/institution/app-downloads`, async ({ request }) => {
    requireAdmin();
    const body = (await request.json()) as { downloads: { platform: AppPlatform; url: string }[] };
    const platforms = ["windows", "macos_arm", "macos_intel", "linux_deb", "linux_appimage"];
    const bad = body.downloads.some((d) => !platforms.includes(d.platform) || !/^https:\/\/[^\s/@]+(\/\S*)?$/.test(d.url.trim()))
      || new Set(body.downloads.map((d) => d.platform)).size !== body.downloads.length;
    if (bad) throw problem(422, "Unprocessable Entity", "the request has invalid fields", [{ field: "downloads", message: "must be an https address" }]);
    const db = getDb();
    db.institution.app_downloads = platforms.flatMap((p) => body.downloads.filter((d) => d.platform === p).map((d) => ({ platform: d.platform, url: d.url.trim() })));
    return HttpResponse.json(db.institution.app_downloads);
  }),
  http.get(`${A}/admin/audit`, ({ request }) => {
    requireAdmin();
    const q = new URL(request.url).searchParams;
    const text = (q.get("q") ?? "").toLowerCase();
    const kind = q.get("kind");
    const days = Number(q.get("days") ?? 7);
    const since = MOCK_NOW.getTime() - days * 86_400_000;
    const matches = (e: ReturnType<typeof getDb>["audit"][number]) =>
      new Date(e.at).getTime() >= since && (!kind || kind === "all" || e.kind === kind) &&
      (!text || `${e.actor?.name ?? ""} ${e.actor?.surname ?? ""} ${e.action} ${e.resource_label ?? ""}`.toLowerCase().includes(text));
    const items = getDb().audit.filter(matches);
    return HttpResponse.json({ items, next_cursor: null });
  }),
];
