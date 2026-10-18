import { http, HttpResponse } from "msw";
import { MOCK_NOW } from "@/lib/clock";
import { sumPoints, toCents } from "@/lib/points";
import type { FormConfig } from "@/lib/api/gaps";
import type { CourseCard } from "@/lib/api/types";
import { configFromWire, configToWire } from "@/lib/api/sheetWire";
import { isoToLocal, localToIso } from "@/lib/format";
import type { CourseInfo, CoursePeople, Item, ServerItemMeta, TemplateSummary } from "@/lib/api/types";
import { at, auditRow, getDb, institutionPublic, uid, type DbCourse, type DbItem } from "../db";
import {
  activeStudents, canView, contentTree, courseById, decimal, dueIn, flatItems, forbidden, formOf, fullName, gradeOf, hasUnpublishedChanges, isPublished,
  isStaff, itemById, itemVisibleToStudent, memberOf, moduleOf, notFound, ownerOf, paged, problem, publicUser, quizOf, requireStaff, requireUser, requireView,
  SESSION_KEY, sessionUser, studentConfig, submissionOf, teacherName, termEnded, termName, userById, workItem, workStatus,
} from "../logic";
import { PASSWORD } from "../seed";

const A = "/api/v1";

export function courseInfo(c: DbCourse): CourseInfo {
  const db = getDb();
  const all = db.members.filter((m) => m.course_id === c.id);
  const graded = flatItems(c, true).filter((i) => i.assignment && toCents(i.assignment.max_points) > 0 && !i.assignment.is_bonus && i.status === "published");
  return {
    course_id: c.id, code: c.code, description: c.description, term_name: termName(c),
    owner_name: ownerOf(c).name, owner_surname: ownerOf(c).surname, owner_email: ownerOf(c).email,
    student_count: all.filter((m) => m.role === "student" && !m.left_on).length,
    former_count: all.filter((m) => m.role === "student" && m.left_on).length,
    assistant_count: all.filter((m) => m.role === "assistant").length,
    updated_at: c.settings_saved_at ?? c.created_at,
    content_updated_at: db.items.filter((i) => i.course_id === c.id).map((i) => i.updated_at).sort().at(-1) ?? null,
    total_points: sumPoints(graded.map((i) => i.assignment!.max_points)), graded_items: graded.length,
    has_grades: db.grades.some((g) => db.items.some((i) => i.id === g.assignment_id && i.course_id === c.id)),
  };
}

function copyContent(fromId: string, toId: string) {
  const db = getDb();
  const idMap = new Map<string, string>();
  for (const m of db.modules.filter((x) => x.course_id === fromId)) {
    const id = uid(`copy:${toId}:${m.id}`);
    idMap.set(m.id, id);
    db.modules.push({ ...m, id, course_id: toId, status: "draft", publish_at: null });
  }
  for (const m of db.modules.filter((x) => x.course_id === toId && x.parent_id)) m.parent_id = idMap.get(m.parent_id!) ?? null;
  for (const i of db.items.filter((x) => x.course_id === fromId)) {
    const copy: DbItem = { ...i, id: uid(`copy:${toId}:${i.id}`), course_id: toId, module_id: idMap.get(i.module_id)!, status: "draft", publish_at: null, unpublished_changes: false, last_published_at: null };
    if (copy.assignment) copy.assignment = { ...copy.assignment, due_at: null, on_deadline: null, accept_late: null, late_until: null };
    db.items.push(copy);
  }
}

function newCourse(title: string, ownerId: string, kind: "regular" | "template", termId: string | null = null): DbCourse {
  const db = getDb();
  const c: DbCourse = {
    id: uid(`course:new:${db.courses.length}:${title}`), kind, title, status: "draft", owner_id: ownerId, term_id: termId, created_at: MOCK_NOW.toISOString(),
    code: "", description: "", settings_saved_at: null, template_updated_at: kind === "template" ? MOCK_NOW.toISOString() : undefined,
  };
  db.courses.push(c);
  db.members.push({ course_id: c.id, user_id: ownerId, role: "teacher", is_owner: true, source: "direct", group_label: null, left_on: null });
  return c;
}

export const publicCourse = ({ code: _c, description: _d, settings_saved_at: _s, template_updated_at: _t, ...c }: DbCourse) => c;

function itemFor(item: DbItem, staff: boolean): Item {
  const { read_minutes: _r, class_on: _o, last_published_at: _l, unpublished_changes: _u, published_body, published_title, ...rest } = item;
  const out: Item = { ...rest };
  if (!staff && item.unpublished_changes) {
    if (published_body) out.body = published_body;
    if (published_title) out.title = published_title;
  }
  if (out.assignment) {
    const type = out.assignment.type;
    const ui = staff ? out.assignment.config : studentConfig(item);
    const wire = configToWire(type, ui) as { fields?: { options?: Record<string, unknown>[]; has_answers?: boolean; rules?: { exclusive?: boolean } }[] };
    // The facts the server adds to a form when it reads it (ADR-0051 point 5).
    if (type === "form" && wire.fields) {
      (ui as unknown as FormConfig).fields.forEach((f, i) => {
        const w = wire.fields![i]!;
        if (staff) w.has_answers = f.has_answers;
        if (f.type === "choice" && w.rules?.exclusive) f.options.forEach((o, k) => { w.options![k] = { ...w.options![k], taken: !!o.taken_by, ...(staff ? { taken_by: o.taken_by } : {}) }; });
      });
    }
    out.assignment = { ...out.assignment, config: wire as unknown as Record<string, never> };
  }
  return out;
}

/** Cards for the course list. Students get progress, staff get a to-do list. */
function cards(userId: string): CourseCard[] {
  const db = getDb();
  const user = userById(userId);
  const mine = db.courses.filter((c) => c.kind === "regular" && memberOf(c.id, userId) && (user.role !== "student" || c.status !== "draft"));
  return mine.map((c): CourseCard => {
    const m = memberOf(c.id, userId)!;
    const staff = isStaff(user, c);
    const myRole = c.owner_id === userId ? "owner" : m.role;
    const base = {
      course: publicCourse(c), code: c.code, term_name: termName(c), term_ended: termEnded(c), owner_name: ownerOf(c).name, owner_surname: ownerOf(c).surname, my_role: myRole,
      student_count: activeStudents(c.id).length, assistant_count: db.members.filter((x) => x.course_id === c.id && x.role === "assistant").length,
    } as const;
    if (!staff) {
      const asg = flatItems(c, false).filter((i) => i.assignment && i.assignment.type !== "no_submission");
      const statuses = asg.map((i) => workStatus(i, userId));
      const upcoming = asg
        .filter((i) => i.assignment!.due_at && new Date(i.assignment!.due_at) >= MOCK_NOW && ["not_started", "draft"].includes(workStatus(i, userId)))
        .sort((a, b) => a.assignment!.due_at!.localeCompare(b.assignment!.due_at!))[0];
      return {
        ...base, todo: [],
        progress: { submitted: statuses.filter((s) => s === "submitted" || s === "graded").length, total: asg.length, graded: statuses.filter((s) => s === "graded").length },
        next: upcoming ? { assignment_id: upcoming.id, title: upcoming.title, due_at: upcoming.assignment!.due_at! } : null,
      };
    }
    const todo: CourseCard["todo"] = [];
    const items = db.items.filter((i) => i.course_id === c.id && i.assignment);
    let toGrade = 0;
    let drafts = 0;
    let draftNote = "";
    for (const i of items) {
      for (const s of activeStudents(c.id)) {
        const sub = submissionOf(i.id, s.user_id);
        const g = gradeOf(i.id, s.user_id);
        if (sub && sub.attempts.length > 0 && toCents(i.assignment!.max_points) > 0 && (!g || g.draft_score === null)) toGrade++;
        if (g && hasUnpublishedChanges(g)) { drafts++; if (i.assignment!.type === "quiz") draftNote = i.title; }
      }
    }
    if (toGrade) todo.push({ kind: "to_grade", count: toGrade });
    if (drafts) todo.push({ kind: "draft_grades", count: drafts, auto_scored: !!draftNote });
    const unpub = db.items.filter((i) => i.course_id === c.id && i.status === "draft" && moduleOf(i).status === "published");
    if (unpub.length) todo.push({ kind: "unpublished_items", count: unpub.length, titles: unpub.slice(0, 3).map((i) => i.title) });
    const nextDue = items.filter((i) => i.assignment!.due_at && new Date(i.assignment!.due_at) >= MOCK_NOW).sort((a, b) => a.assignment!.due_at!.localeCompare(b.assignment!.due_at!))[0];
    if (nextDue) todo.push({ kind: "next_deadline", assignment_id: nextDue.id, title: nextDue.title, due_at: nextDue.assignment!.due_at! });
    return { ...base, progress: null, next: null, todo };
  });
}

export function itemMeta(c: DbCourse, userId: string, staff: boolean): Record<string, ServerItemMeta> {
  const out: Record<string, ServerItemMeta> = {};
  for (const i of flatItems(c, staff)) {
    const meta: ServerItemMeta = {};
    if (i.read_minutes) meta.read_minutes = i.read_minutes;
    if (i.class_on) meta.class_on = isoToLocal(i.class_on).slice(0, 10);
    if (i.assignment && !staff) {
      const w = workItem(i, userId);
      meta.status = w.status; meta.score = w.score; meta.draft_saved_at = w.draft_saved_at; meta.published_at = w.published_at;
      meta.attempts_used = submissionOf(i.id, userId)?.attempts.length ?? 0;
    }
    if (staff) {
      if (i.unpublished_changes) meta.edited = true;
      if (i.assignment) {
        const students = activeStudents(c.id);
        meta.graded_total = students.length;
        meta.submitted = students.filter((s) => (submissionOf(i.id, s.user_id)?.attempts.length ?? 0) > 0).length;
        meta.to_grade = students.filter((s) => (submissionOf(i.id, s.user_id)?.attempts.length ?? 0) > 0 && gradeOf(i.id, s.user_id)?.draft_score == null).length;
        meta.draft_grades = students.filter((s) => { const g = gradeOf(i.id, s.user_id); return !!g && hasUnpublishedChanges(g); }).length;
      }
    }
    out[i.id] = meta;
  }
  return out;
}

export const coreHandlers = [
  // ---- auth ----
  http.post(`${A}/auth/login`, async ({ request }) => {
    const body = (await request.json()) as { email?: string; password?: string };
    const u = getDb().users.find((x) => x.email === (body.email ?? "").trim().toLowerCase());
    if (!u || u.status !== "active" || !u.password || u.password !== body.password) throw problem(401, "Unauthorized");
    localStorage.setItem(SESSION_KEY, u.id);
    u.last_sign_in = MOCK_NOW.toISOString();
    return HttpResponse.json(publicUser(u), { headers: { "Cache-Control": "no-store" } });
  }),
  http.post(`${A}/auth/logout`, () => {
    localStorage.removeItem(SESSION_KEY);
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/me`, () => HttpResponse.json(publicUser(requireUser()))),
  http.post(`${A}/me/password`, async ({ request }) => {
    const u = requireUser();
    const b = (await request.json()) as { current_password?: string; new_password?: string };
    if (b.current_password !== u.password) throw problem(422, "Validation failed", undefined, [{ field: "current_password", message: "That is not your current password" }]);
    if ((b.new_password ?? "").length < 10) throw problem(422, "Validation failed", undefined, [{ field: "new_password", message: "Use at least 10 characters" }]);
    u.password = b.new_password!;
    u.must_change_password = false;
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/me/sessions`, ({ request }) => {
    requireUser();
    return paged(getDb().sessions, request);
  }),
  http.delete(`${A}/me/sessions/:id`, ({ params }) => {
    requireUser();
    const db = getDb();
    db.sessions = db.sessions.filter((s) => s.id !== params.id || s.current);
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/institution`, () => HttpResponse.json(institutionPublic(getDb().institution))),
  http.get(`${A}/editor/profiles`, () => {
    requireUser();
    const limits = { max_document_bytes: 1_048_576, max_depth: 3, max_table_rows: 50, max_table_columns: 10, max_code_bytes: 65_536, max_formula_bytes: 4096, max_href_chars: 2048 };
    const full = ["headings", "text_styles", "links", "lists", "quote", "divider", "table", "code", "math", "mermaid", "video"] as const;
    return HttpResponse.json({
      profiles: [
        { name: "full", features: [...full], single_block: false, limits },
        { name: "student", features: ["text_styles", "links", "lists", "code", "math"], single_block: false, limits },
        { name: "question", features: ["text_styles", "code", "math"], single_block: false, limits },
        { name: "option", features: ["text_styles", "math"], single_block: true, limits },
      ],
      video_providers: ["youtube", "vimeo"],
    });
  }),

  // ---- courses ----
  http.get(`${A}/me/courses`, ({ request }) => {
    const u = requireUser();
    const db = getDb();
    const list = db.courses.filter((c) => memberOf(c.id, u.id) && (u.role !== "student" || c.status !== "draft")).map(publicCourse);
    return paged(list, request);
  }),
  http.get(`${A}/me/course-cards`, () => HttpResponse.json({ items: cards(requireUser().id) })),
  http.get(`${A}/me/templates`, () => {
    const u = requireUser();
    const db = getDb();
    const items: TemplateSummary[] = db.courses.filter((c) => c.kind === "template" && c.owner_id === u.id).map((c) => {
      const mods = db.modules.filter((m) => m.course_id === c.id);
      const its = db.items.filter((i) => i.course_id === c.id);
      return { course: publicCourse(c), sections: mods.length, lectures: its.filter((i) => i.kind === "text").length, assignments: its.filter((i) => i.kind === "assignment").length, updated_at: c.template_updated_at ?? c.created_at };
    });
    return HttpResponse.json({ items, next_cursor: null });
  }),
  http.post(`${A}/templates/:id/courses`, async ({ request, params }) => {
    const u = requireUser();
    if (u.role === "student") throw forbidden();
    const tpl = courseById(String(params.id));
    const b = (await request.json()) as { title?: string };
    const c = newCourse((b.title ?? tpl.title).trim() || tpl.title, u.id, "regular");
    copyContent(tpl.id, c.id);
    return HttpResponse.json(publicCourse(c), { status: 201 });
  }),
  http.post(`${A}/courses`, async ({ request }) => {
    const u = requireUser();
    if (u.role === "student") throw forbidden();
    const b = (await request.json()) as { title?: string; term_id?: string | null };
    const title = (b.title ?? "").trim();
    if (!title) throw problem(422, "Validation failed", undefined, [{ field: "title", message: "Give the course a title" }]);
    return HttpResponse.json(publicCourse(newCourse(title, u.id, "regular", b.term_id ?? null)), { status: 201 });
  }),
  http.get(`${A}/courses/:id`, ({ params }) => HttpResponse.json(publicCourse(requireView(String(params.id)).course))),
  http.patch(`${A}/courses/:id`, async ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    const b = (await request.json()) as { title?: string; term_id?: string | null };
    if (b.title !== undefined) {
      if (!b.title.trim()) throw problem(422, "Validation failed", undefined, [{ field: "title", message: "Give the course a title" }]);
      course.title = b.title.trim();
    }
    if (b.term_id !== undefined) course.term_id = b.term_id;
    course.settings_saved_at = MOCK_NOW.toISOString();
    return HttpResponse.json(publicCourse(course));
  }),
  http.get(`${A}/courses/:id/info`, ({ params }) => HttpResponse.json(courseInfo(requireView(String(params.id)).course))),
  http.patch(`${A}/courses/:id/info`, async ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    const b = (await request.json()) as { code?: string; description?: string };
    if (b.code !== undefined) course.code = b.code.trim();
    if (b.description !== undefined) {
      if (b.description.length > 300) throw problem(422, "Validation failed", undefined, [{ field: "description", message: "Use at most 300 characters" }]);
      course.description = b.description;
    }
    course.settings_saved_at = MOCK_NOW.toISOString();
    return HttpResponse.json(courseInfo(course));
  }),
  http.delete(`${A}/courses/:id`, ({ params }) => {
    const { course } = requireStaff(String(params.id));
    const db = getDb();
    if (courseInfo(course).has_grades) throw problem(409, "Conflict", "This course already has grades, so it can’t be deleted. Archive it instead.");
    db.courses = db.courses.filter((c) => c.id !== course.id);
    db.modules = db.modules.filter((m) => m.course_id !== course.id);
    db.items = db.items.filter((i) => i.course_id !== course.id);
    db.members = db.members.filter((m) => m.course_id !== course.id);
    return new HttpResponse(null, { status: 204 });
  }),
  http.post(`${A}/courses/:id/publish`, ({ params }) => {
    const { course } = requireStaff(String(params.id));
    course.status = "published";
    return HttpResponse.json(publicCourse(course));
  }),
  http.post(`${A}/courses/:id/archive`, ({ params }) => {
    const { course } = requireStaff(String(params.id));
    course.status = "archived";
    return HttpResponse.json(publicCourse(course));
  }),
  http.post(`${A}/courses/:id/restore`, ({ params }) => {
    const { course } = requireStaff(String(params.id));
    course.status = "published";
    return HttpResponse.json(publicCourse(course));
  }),
  http.post(`${A}/courses/:id/template`, ({ params }) => {
    const { course, user } = requireStaff(String(params.id));
    const tpl = newCourse(course.title, user.id, "template");
    copyContent(course.id, tpl.id);
    return HttpResponse.json(publicCourse(tpl), { status: 201 });
  }),
  http.put(`${A}/courses/:id/owner`, async ({ request, params }) => {
    const { course, user } = requireStaff(String(params.id));
    if (course.owner_id !== user.id && user.role !== "admin") throw forbidden();
    const b = (await request.json()) as { user_id: string };
    const target = userById(b.user_id);
    if (target.role === "student") throw problem(422, "Validation failed", undefined, [{ field: "user_id", message: "The owner must be a teacher or an admin" }]);
    const db = getDb();
    for (const m of db.members.filter((x) => x.course_id === course.id)) m.is_owner = m.user_id === target.id;
    if (!memberOf(course.id, target.id)) db.members.push({ course_id: course.id, user_id: target.id, role: "teacher", is_owner: true, source: "direct", group_label: null, left_on: null });
    const previous = userById(course.owner_id);
    course.owner_id = target.id;
    db.audit.unshift(auditRow(`${db.audit.length}:transfer`, MOCK_NOW.toISOString(), user, "course.owner.transfer", { type: "course", id: course.id, label: `${course.code} ${course.title}` }, { from: previous.id, to: target.id }));
    return HttpResponse.json(publicCourse(course));
  }),
  http.get(`${A}/courses/:id/content`, ({ params }) => {
    const { course, staff } = requireView(String(params.id));
    return HttpResponse.json(contentTree(course, staff));
  }),
  http.get(`${A}/courses/:id/item-meta`, ({ params }) => {
    const { course, user, staff } = requireView(String(params.id));
    return HttpResponse.json({ items: itemMeta(course, user.id, staff) });
  }),
  http.get(`${A}/courses/:id/members`, ({ request, params }) => {
    const { course } = requireView(String(params.id));
    const list = getDb().members.filter((m) => m.course_id === course.id && !m.left_on).sort((a, b) => a.user_id.localeCompare(b.user_id));
    return paged(list.map(({ group_label: _g, left_on: _l, ...m }) => m), request);
  }),
  http.put(`${A}/courses/:id/members/:uid`, async ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    const b = (await request.json()) as { role: "teacher" | "assistant" | "student" };
    const db = getDb();
    const target = userById(String(params.uid));
    let m = memberOf(course.id, target.id);
    if (m?.is_owner) throw problem(409, "Conflict", "The owner cannot change role");
    if (!m) {
      m = { course_id: course.id, user_id: target.id, role: b.role, is_owner: false, source: "direct", group_label: null, left_on: null };
      db.members.push(m);
    } else { m.role = b.role; m.left_on = null; }
    const { group_label: _g, left_on: _l, ...pub } = m;
    return HttpResponse.json(pub);
  }),
  http.delete(`${A}/courses/:id/members/:uid`, ({ params }) => {
    const { course } = requireStaff(String(params.id));
    const m = memberOf(course.id, String(params.uid));
    if (!m) throw notFound();
    if (m.is_owner) throw problem(409, "Conflict", "The owner cannot be removed");
    if (m.source === "group") throw problem(409, "Conflict", "Remove them from the group instead");
    const db = getDb();
    db.members = db.members.filter((x) => x !== m);
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/courses/:id/people`, ({ params }) => {
    const { course, staff } = requireView(String(params.id));
    const db = getDb();
    const people = db.members.filter((m) => m.course_id === course.id && (staff || !m.left_on)).map((m) => {
      const u = userById(m.user_id);
      return {
        user_id: u.id, name: u.name, surname: u.surname, email: staff ? u.email : null, role: m.role, is_owner: m.is_owner, source: m.source,
        group: m.group_label, former: !!m.left_on,
      };
    }).sort((a, b) => Number(b.role !== "student") - Number(a.role !== "student") || a.surname.localeCompare(b.surname));
    const groups = staff ? db.links.filter((l) => l.course_id === course.id).map((l) => {
      const g = db.groups.find((x) => x.id === l.group_id)!;
      const sub = db.subgroups.find((x) => x.id === l.subgroup_id);
      const count = db.groupMembers.filter((gm) => gm.group_id === l.group_id && (!l.subgroup_id || gm.subgroup_id === l.subgroup_id)).length;
      return { link_id: l.id, group_name: g.name, subgroup_name: sub?.name ?? null, student_count: count, linked_at: l.linked_on };
    }) : [];
    return HttpResponse.json({ people, groups } satisfies CoursePeople);
  }),
  http.get(`${A}/courses/:id/candidates`, ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    const q = (new URL(request.url).searchParams.get("q") ?? "").toLowerCase().trim();
    const items = getDb().users
      .filter((u) => u.status === "active" && !memberOf(course.id, u.id) && (!q || `${u.name} ${u.surname} ${u.email}`.toLowerCase().includes(q)))
      .slice(0, 8)
      .map((u) => ({ user_id: u.id, name: u.name, surname: u.surname, email: u.email, role: u.role }));
    return HttpResponse.json({ items });
  }),
  http.get(`${A}/courses/:id/groups`, ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    return paged(getDb().links.filter((l) => l.course_id === course.id).map(({ linked_on: _l, ...l }) => l), request);
  }),
  http.post(`${A}/courses/:id/groups`, async ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    const b = (await request.json()) as { group_id: string; subgroup_id?: string | null };
    const db = getDb();
    const existing = db.links.find((l) => l.course_id === course.id && l.group_id === b.group_id && (l.subgroup_id ?? null) === (b.subgroup_id ?? null));
    if (existing) { const { linked_on: _l, ...pub } = existing; return HttpResponse.json(pub, { status: 200 }); }
    const link = { id: uid(`link:${course.id}:${b.group_id}:${b.subgroup_id ?? ""}`), course_id: course.id, group_id: b.group_id, subgroup_id: b.subgroup_id ?? null, created_at: MOCK_NOW.toISOString(), linked_on: MOCK_NOW.toISOString() };
    db.links.push(link);
    const group = db.groups.find((g) => g.id === b.group_id);
    for (const gm of db.groupMembers.filter((x) => x.group_id === b.group_id && (!b.subgroup_id || x.subgroup_id === b.subgroup_id))) {
      if (!memberOf(course.id, gm.user_id)) db.members.push({ course_id: course.id, user_id: gm.user_id, role: "student", is_owner: false, source: "group", group_label: db.subgroups.find((s) => s.id === gm.subgroup_id)?.name ?? group?.name ?? null, left_on: null });
    }
    const { linked_on: _l, ...pub } = link;
    return HttpResponse.json(pub, { status: 201 });
  }),
  http.delete(`${A}/courses/:id/groups/:link`, ({ params }) => {
    const { course } = requireStaff(String(params.id));
    const db = getDb();
    const link = db.links.find((l) => l.id === params.link && l.course_id === course.id);
    if (!link) throw notFound();
    db.links = db.links.filter((l) => l !== link);
    const still = new Set(db.links.filter((l) => l.course_id === course.id).flatMap((l) => db.groupMembers.filter((gm) => gm.group_id === l.group_id && (!l.subgroup_id || gm.subgroup_id === l.subgroup_id)).map((gm) => gm.user_id)));
    db.members = db.members.filter((m) => !(m.course_id === course.id && m.source === "group" && !still.has(m.user_id)));
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/courses/:id/group-options`, ({ request, params }) => {
    requireStaff(String(params.id));
    const db = getDb();
    return paged(db.groups.map((g) => ({ id: g.id, name: g.name, subgroups: db.subgroups.filter((s) => s.group_id === g.id).map((s) => ({ id: s.id, name: s.name })) })), request);
  }),

  // ---- content ----
  http.get(`${A}/items/:id`, ({ params }) => {
    const user = requireUser();
    const item = itemById(String(params.id));
    const course = courseById(item.course_id);
    if (!canView(user, course)) throw notFound();
    const staff = isStaff(user, course);
    if (!staff && !itemVisibleToStudent(item, course)) throw notFound();
    return HttpResponse.json(itemFor(item, staff));
  }),
  http.patch(`${A}/items/:id`, async ({ request, params }) => {
    const item = itemById(String(params.id));
    requireStaff(item.course_id);
    const b = (await request.json()) as Record<string, unknown>;
    if (item.status === "published" && (b.body !== undefined || b.title !== undefined) && !item.unpublished_changes) {
      item.published_body = item.body;
      item.published_title = item.title;
      item.unpublished_changes = true;
    }
    if (typeof b.title === "string") {
      if (!b.title.trim()) throw problem(422, "Validation failed", undefined, [{ field: "title", message: "Give it a title" }]);
      item.title = b.title.trim();
    }
    if (b.body) item.body = b.body as Item["body"];
    if (item.assignment) {
      const a = item.assignment;
      if (b.config) a.config = configFromWire(a.type, b.config as Record<string, unknown>);
      if (b.max_points !== undefined) a.max_points = decimal(b.max_points, "max_points");
      if ("due_at" in b) { a.due_at = (b.due_at as string | null) ?? null; if (!a.due_at) { a.on_deadline = null; a.accept_late = null; a.late_until = null; } }
      if ("on_deadline" in b) a.on_deadline = b.on_deadline as typeof a.on_deadline;
      if ("accept_late" in b) a.accept_late = b.accept_late as boolean | null;
      if ("late_until" in b) a.late_until = b.late_until as string | null;
      if ("max_attempts" in b) a.max_attempts = b.max_attempts as number | null;
    }
    if ("class_on" in b) item.class_on = b.class_on ? localToIso(`${b.class_on as string}T12:00`) : null;
    if ("publish_at" in b) item.publish_at = b.publish_at as string | null;
    item.updated_at = MOCK_NOW.toISOString();
    return HttpResponse.json(itemFor(item, true));
  }),
  http.delete(`${A}/items/:id`, ({ params }) => {
    const item = itemById(String(params.id));
    requireStaff(item.course_id);
    const db = getDb();
    if (db.grades.some((g) => g.assignment_id === item.id && (g.published_score !== null || g.draft_score !== null))) throw problem(409, "Conflict", "This assignment has grades, so it can’t be deleted");
    db.items = db.items.filter((i) => i !== item);
    return new HttpResponse(null, { status: 204 });
  }),
  http.post(`${A}/items/:id/publish`, ({ params }) => {
    const item = itemById(String(params.id));
    requireStaff(item.course_id);
    item.status = "published";
    item.last_published_at ??= MOCK_NOW.toISOString();
    return HttpResponse.json(itemFor(item, true));
  }),
  http.get(`${A}/items/:id/revision`, ({ params }) => {
    const item = itemById(String(params.id));
    requireStaff(item.course_id);
    return HttpResponse.json({ has_unpublished_changes: item.unpublished_changes, last_published_at: item.last_published_at, size_bytes: JSON.stringify(item.body).length });
  }),
  http.post(`${A}/items/:id/revision/publish`, ({ params }) => {
    const item = itemById(String(params.id));
    requireStaff(item.course_id);
    item.unpublished_changes = false;
    item.published_body = undefined;
    item.published_title = undefined;
    item.last_published_at = MOCK_NOW.toISOString();
    return HttpResponse.json({ has_unpublished_changes: false, last_published_at: item.last_published_at, size_bytes: JSON.stringify(item.body).length });
  }),
  http.post(`${A}/courses/:id/modules`, async ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    const b = (await request.json()) as { title?: string; parent_id?: string | null };
    const title = (b.title ?? "").trim();
    if (!title) throw problem(422, "Validation failed", undefined, [{ field: "title", message: "Give the section a title" }]);
    const db = getDb();
    const sibs = db.modules.filter((m) => m.course_id === course.id && (m.parent_id ?? null) === (b.parent_id ?? null));
    const m = { id: uid(`module:new:${db.modules.length}:${title}`), course_id: course.id, parent_id: b.parent_id ?? null, position: sibs.length + 1, title, status: "draft" as const, publish_at: null, created_at: MOCK_NOW.toISOString(), updated_at: MOCK_NOW.toISOString() };
    db.modules.push(m);
    return HttpResponse.json(m, { status: 201 });
  }),
  http.put(`${A}/courses/:id/modules/order`, async ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    const b = (await request.json()) as { ids: string[]; parent_id?: string | null };
    b.ids.forEach((id, i) => { const m = getDb().modules.find((x) => x.id === id && x.course_id === course.id); if (m) m.position = i + 1; });
    return new HttpResponse(null, { status: 204 });
  }),
  http.patch(`${A}/modules/:id`, async ({ request, params }) => {
    const m = getDb().modules.find((x) => x.id === params.id);
    if (!m) throw notFound();
    requireStaff(m.course_id);
    const b = (await request.json()) as { title?: string; publish_at?: string | null };
    if (b.title !== undefined) m.title = b.title.trim() || m.title;
    if ("publish_at" in b) m.publish_at = b.publish_at ?? null;
    return HttpResponse.json(m);
  }),
  http.delete(`${A}/modules/:id`, ({ params }) => {
    const db = getDb();
    const m = db.modules.find((x) => x.id === params.id);
    if (!m) throw notFound();
    requireStaff(m.course_id);
    const ids = new Set([m.id, ...db.modules.filter((x) => x.parent_id === m.id).map((x) => x.id)]);
    if (db.items.some((i) => ids.has(i.module_id) && db.grades.some((g) => g.assignment_id === i.id))) throw problem(409, "Conflict", "This section has graded work, so it can’t be deleted");
    db.items = db.items.filter((i) => !ids.has(i.module_id));
    db.modules = db.modules.filter((x) => !ids.has(x.id));
    return new HttpResponse(null, { status: 204 });
  }),
  http.post(`${A}/modules/:id/publish`, async ({ request, params }) => {
    const db = getDb();
    const m = db.modules.find((x) => x.id === params.id);
    if (!m) throw notFound();
    requireStaff(m.course_id);
    const b = (await request.json()) as { cascade?: boolean };
    if (typeof b.cascade !== "boolean") throw problem(422, "Validation failed", undefined, [{ field: "cascade", message: "Choose whether to publish what is inside" }]);
    m.status = "published";
    if (b.cascade) {
      const ids = [m.id, ...db.modules.filter((x) => x.parent_id === m.id).map((x) => x.id)];
      for (const s of db.modules.filter((x) => ids.includes(x.id))) s.status = "published";
      for (const i of db.items.filter((x) => ids.includes(x.module_id) && x.status === "draft")) { i.status = "published"; i.last_published_at = MOCK_NOW.toISOString(); }
    }
    return HttpResponse.json(m);
  }),
  http.post(`${A}/modules/:id/items`, async ({ request, params }) => {
    const db = getDb();
    const m = db.modules.find((x) => x.id === params.id);
    if (!m) throw notFound();
    requireStaff(m.course_id);
    const b = (await request.json()) as { kind: "text" | "assignment"; title: string; body?: unknown; assignment?: Record<string, unknown> };
    const title = (b.title ?? "").trim();
    if (!title) throw problem(422, "Validation failed", undefined, [{ field: "title", message: "Give it a title" }]);
    const id = uid(`item:new:${db.items.length}:${title}`);
    const sibs = db.items.filter((i) => i.module_id === m.id);
    const a = b.assignment;
    const item: DbItem = {
      id, module_id: m.id, course_id: m.course_id, position: sibs.length + 1, kind: b.kind, title, body: (b.body ?? { v: 1, blocks: [] }) as Item["body"], status: "draft", publish_at: null,
      created_at: MOCK_NOW.toISOString(), updated_at: MOCK_NOW.toISOString(), last_published_at: null, unpublished_changes: false,
      ...(b.kind === "assignment" ? {
        assignment: {
          type: String(a?.type ?? "text"), config: configFromWire(String(a?.type ?? "text"), (a?.config as Record<string, unknown>) ?? {}), max_points: decimal(a?.max_points ?? "10", "max_points"), is_bonus: false,
          due_at: (a?.due_at as string | null) ?? null, on_deadline: (a?.on_deadline as "keep_draft" | "auto_submit" | null) ?? null,
          accept_late: (a?.accept_late as boolean | null) ?? null, late_until: null, max_attempts: a?.max_attempts === undefined ? 1 : (a.max_attempts as number | null),
        },
      } : {}),
    };
    db.items.push(item);
    return HttpResponse.json(itemFor(item, true), { status: 201 });
  }),
  http.put(`${A}/modules/:id/items/order`, async ({ request, params }) => {
    const db = getDb();
    const m = db.modules.find((x) => x.id === params.id);
    if (!m) throw notFound();
    requireStaff(m.course_id);
    const b = (await request.json()) as { ids: string[] };
    b.ids.forEach((id, i) => { const it = db.items.find((x) => x.id === id && x.module_id === m.id); if (it) it.position = i + 1; });
    return new HttpResponse(null, { status: 204 });
  }),
];

export { PASSWORD, at, dueIn, formOf, isPublished, quizOf, sessionUser };
