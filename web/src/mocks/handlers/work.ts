import { http, HttpResponse } from "msw";
import { MOCK_NOW } from "@/lib/clock";
import { isoToLocal } from "@/lib/format";
import { toCents } from "@/lib/points";
import type { AnswerMap } from "@/lib/api/gaps";
import type { MyWork, SearchHit, TeachingOverview } from "@/lib/api/types";
import { answersFromWire, answersToWire } from "@/lib/api/sheetWire";
import { getDb, uid, type DbSubmission } from "../db";
import {
  activeStudents, autoScore, canView, courseById, decimal, dueIn, flatItems, forbidden, formOf, fullName, gradeOf, hasUnpublishedChanges, isPublished, isStaff,
  itemById, itemVisibleToStudent, memberOf, notFound, paged, problem, quizOf, requireStaff, requireUser, requireView, scoreQuestion, submissionOf, userById, workItem,
} from "../logic";
import { courseInfo } from "./core";

const A = "/api/v1";

function assignmentFor(id: string) {
  const item = itemById(id);
  if (!item.assignment) throw notFound();
  return item;
}

function viewAssignment(id: string) {
  const user = requireUser();
  const item = assignmentFor(id);
  const course = courseById(item.course_id);
  if (!canView(user, course)) throw notFound();
  const staff = isStaff(user, course);
  if (!staff && !itemVisibleToStudent(item, course)) throw notFound();
  return { user, item, course, staff };
}

type MockItem = ReturnType<typeof assignmentFor>;

/** The content of a quiz or form submission as the server stores it, from what the mock keeps. */
function wireContent(item: MockItem, content: unknown): unknown {
  const a = item.assignment!;
  if (a.type !== "quiz" && a.type !== "form") return content;
  const answers = (content as { answers?: AnswerMap } | null)?.answers ?? {};
  return { answers: answersToWire(a.type, a.config, answers) };
}

/** What the mock keeps of the content a client sent. */
function uiContent(item: MockItem, content: unknown): unknown {
  const a = item.assignment!;
  if (a.type !== "quiz" && a.type !== "form") return content;
  return { answers: answersFromWire(a.type, a.config, content) };
}

function staffAssignment(id: string) {
  const item = assignmentFor(id);
  const { course, user } = requireStaff(item.course_id);
  return { item, course, user };
}

function latestAnswers(sub: DbSubmission | undefined): Record<string, string | string[]> {
  const content = sub?.attempts[0]?.content as { answers?: Record<string, string | string[]> } | undefined;
  return content?.answers ?? {};
}

export function toGradeList(courseIds: string[]) {
  const db = getDb();
  const out: TeachingOverview["to_grade"] = [];
  for (const cid of courseIds) {
    const c = courseById(cid);
    const info = courseInfo(c);
    for (const item of db.items.filter((i) => i.course_id === cid && i.assignment && i.assignment.type !== "no_submission" && toCents(i.assignment.max_points) > 0)) {
      for (const m of activeStudents(cid)) {
        const sub = submissionOf(item.id, m.user_id);
        const g = gradeOf(item.id, m.user_id);
        if (!sub || sub.attempts.length === 0 || (g && g.draft_score !== null)) continue;
        const u = userById(m.user_id);
        const a = sub.attempts[0]!;
        out.push({
          student_id: u.id, student_name: u.name, student_surname: u.surname, assignment_id: item.id, assignment_title: item.title, course_id: cid, course_code: info.code,
          course_title: c.title, submitted_at: a.submitted_at, attempt: a.attempt, max_attempts: item.assignment!.max_attempts ?? null,
        });
      }
    }
  }
  return out.sort((a, b) => a.submitted_at.localeCompare(b.submitted_at));
}

/** All the text in a block document: what a search looks through. */
function textIn(node: unknown): string {
  if (typeof node === "string") return "";
  if (Array.isArray(node)) return node.map(textIn).join(" ");
  if (node && typeof node === "object") {
    return Object.entries(node as Record<string, unknown>)
      .map(([k, v]) => (typeof v === "string" ? (k === "text" || k === "source" ? v : "") : textIn(v)))
      .join(" ");
  }
  return "";
}

function snippet(text: string, q: string): string | null {
  const flat = text.replace(/\s+/g, " ").trim();
  const at = flat.toLowerCase().indexOf(q);
  if (at < 0) return null;
  const from = Math.max(0, at - 40);
  const to = Math.min(flat.length, at + q.length + 80);
  return `${from > 0 ? "…" : ""}${flat.slice(from, to)}${to < flat.length ? "…" : ""}`;
}

export const workHandlers = [
  // Search over the courses and materials the caller can see (ADR-0052).
  http.get(`${A}/search`, ({ request }) => {
    const user = requireUser();
    const q = (new URL(request.url).searchParams.get("q") ?? "").trim().toLowerCase();
    if (q.length < 2) return HttpResponse.json({ items: [], next_cursor: null });
    const db = getDb();
    const hits: SearchHit[] = [];
    for (const course of db.courses.filter((c) => c.kind === "regular" && canView(user, c))) {
      const staff = isStaff(user, course);
      const base = { course_id: course.id, course_title: course.title, course_code: course.code };
      if (`${course.title} ${course.code} ${course.description}`.toLowerCase().includes(q)) {
        hits.push({ ...base, kind: "course", id: course.id, title: course.title, snippet: snippet(course.description, q), due_at: null, status: course.status === "draft" ? "draft" : "published" });
      }
      for (const item of flatItems(course, staff)) {
        // A student keeps reading the published text while there are unpublished edits.
        const body = !staff && item.unpublished_changes ? item.published_body : item.body;
        const title = !staff && item.published_title ? item.published_title : item.title;
        const text = textIn(body);
        if (!`${title} ${text}`.toLowerCase().includes(q)) continue;
        hits.push({
          ...base, kind: item.assignment ? "assignment" : "lecture", id: item.id, title, snippet: snippet(text, q),
          due_at: item.assignment?.due_at ?? null, status: item.status,
        });
      }
    }
    // Titles first, then text matches; courses before their items.
    const rank = (h: SearchHit) => (h.title.toLowerCase().includes(q) ? 0 : 1) + (h.kind === "course" ? 0 : 0.5);
    hits.sort((a, b) => rank(a) - rank(b));
    return HttpResponse.json({ items: hits.slice(0, 50), next_cursor: null });
  }),

  // ---- own work ----
  http.get(`${A}/assignments/:id/submission`, ({ params }) => {
    const { user } = viewAssignment(String(params.id));
    const sub = submissionOf(String(params.id), user.id);
    const item = itemById(String(params.id));
    return HttpResponse.json({
      attempts: (sub?.attempts ?? []).map((a) => ({ ...a, content: wireContent(item, a.content) })),
      draft: sub?.draft ? { ...sub.draft, content: wireContent(item, sub.draft.content) } : null,
      next_cursor: null,
    });
  }),
  http.put(`${A}/assignments/:id/submission/draft`, async ({ request, params }) => {
    const { user, item, staff } = viewAssignment(String(params.id));
    if (staff) throw forbidden();
    const a = item.assignment!;
    if (a.type === "no_submission") throw problem(409, "Conflict", "Nothing to submit here");
    if (dueIn(item).closed) throw problem(409, "Conflict", "This assignment is closed");
    const sub = submissionOf(item.id, user.id);
    if (a.max_attempts != null && (sub?.attempts.length ?? 0) >= a.max_attempts) throw problem(409, "Conflict", "You have used all attempts");
    const b = (await request.json()) as { content: unknown };
    const db = getDb();
    let s = sub;
    if (!s) { s = { assignment_id: item.id, student_id: user.id, draft: null, attempts: [] }; db.submissions.push(s); }
    s.draft = { content: uiContent(item, b.content), updated_at: MOCK_NOW.toISOString() };
    return HttpResponse.json({ ...s.draft, content: wireContent(item, s.draft.content) });
  }),
  http.delete(`${A}/assignments/:id/submission/draft`, ({ params }) => {
    const { user } = viewAssignment(String(params.id));
    const s = submissionOf(String(params.id), user.id);
    if (s) s.draft = null;
    return new HttpResponse(null, { status: 204 });
  }),
  http.post(`${A}/assignments/:id/submission/submit`, ({ params }) => {
    const { user, item, staff } = viewAssignment(String(params.id));
    if (staff) throw forbidden();
    const s = submissionOf(item.id, user.id);
    if (!s?.draft) throw problem(409, "Conflict", "Save a draft first");
    const a = item.assignment!;
    if (dueIn(item).closed) throw problem(409, "Conflict", "This assignment is closed");
    if (a.max_attempts != null && s.attempts.length >= a.max_attempts) throw problem(409, "Conflict", "You have used all attempts");
    if (a.type === "form") {
      const answers = (s.draft.content as { answers?: Record<string, string | string[]> }).answers ?? {};
      const missing = formOf(item).fields.filter((f) => f.required && (!answers[f.id] || (Array.isArray(answers[f.id]) && (answers[f.id] as string[]).length === 0)));
      if (missing.length) throw problem(422, "Validation failed", undefined, missing.map((f) => ({ field: `content.answers.${f.id}`, message: "is required" })));
    }
    const attempt = { id: uid(`attempt:${item.id}:${user.id}:${s.attempts.length + 1}:${Date.now()}`), attempt: s.attempts.length + 1, content: s.draft.content, submitted_at: MOCK_NOW.toISOString(), is_late: !!a.due_at && new Date(a.due_at) < MOCK_NOW };
    s.attempts.unshift(attempt);
    s.draft = null;
    if (a.type === "quiz") {
      const q = quizOf(item);
      const hasLong = q.questions.some((x) => x.type === "long");
      if (!hasLong) {
        let g = gradeOf(item.id, user.id);
        const score = autoScore(item, latestAnswers(s));
        const db = getDb();
        if (!g) { g = { assignment_id: item.id, student_id: user.id, draft_score: null, draft_feedback: null, published_score: null, published_feedback: null, published_at: null, history: [] }; db.grades.push(g); }
        g.draft_score = score;
        g.history.unshift({ action: "save_draft", score, feedback: null, actor_id: null, at: MOCK_NOW.toISOString() });
      }
    }
    return HttpResponse.json({ ...attempt, content: wireContent(item, attempt.content) }, { status: 201 });
  }),
  http.get(`${A}/assignments/:id/result`, ({ params }) => {
    const { user, item } = viewAssignment(String(params.id));
    const g = gradeOf(item.id, user.id);
    if (!isPublished(g)) throw notFound();
    const q = quizOf(item);
    const stored = getDb().quizResults[item.id];
    const answers = latestAnswers(submissionOf(item.id, user.id));
    const wired = answersToWire("quiz", item.assignment!.config, answers);
    const result = {
      score: g!.published_score ?? "0.00", max_points: item.assignment!.max_points, published_at: g!.published_at!, feedback: g!.published_feedback,
      attempt: submissionOf(item.id, user.id)?.attempts[0]?.attempt ?? 1,
      questions: q.questions.map((x) => ({
        id: x.id, points: x.points, got: x.type === "long" ? null : (stored?.[x.id]?.got ?? scoreQuestion(x, answers[x.id]) ?? "0.00"),
        answer: wired[x.id] ?? null,
        ...(q.show_correct ? { correct: x.type === "single" || x.type === "multi" ? (x.correct ?? []) : x.type === "number" ? x.answer : x.type === "short" ? [x.answer] : null } : {}),
      })),
    };
    return HttpResponse.json(result);
  }),
  http.get(`${A}/courses/:id/my-grades`, ({ params }) => {
    const { course, user } = requireView(String(params.id));
    const items = flatItems(course, false).filter((i) => i.assignment && isPublished(gradeOf(i.id, user.id))).map((i) => {
      const g = gradeOf(i.id, user.id)!;
      return { assignment_id: i.id, title: i.title, max_points: i.assignment!.max_points, is_bonus: i.assignment!.is_bonus, score: g.published_score, feedback: g.published_feedback, published_at: g.published_at! };
    });
    return HttpResponse.json({ items });
  }),
  http.get(`${A}/me/work`, () => {
    const user = requireUser();
    const db = getDb();
    const mine = db.courses.filter((c) => c.kind === "regular" && c.status !== "draft" && memberOf(c.id, user.id)?.role === "student");
    const work: MyWork = {
      courses: mine.map((c) => ({ id: c.id, code: c.code, title: c.title, term_name: db.terms.find((t) => t.id === c.term_id)?.name ?? null, owner_name: userById(c.owner_id).name, owner_surname: userById(c.owner_id).surname })),
      items: mine.flatMap((c) => flatItems(c, false).filter((i) => i.assignment).map((i) => { const w = workItem(i, user.id); return { ...w, class_on: w.class_on ? isoToLocal(w.class_on).slice(0, 10) : null }; })),
    };
    return HttpResponse.json(work);
  }),

  // ---- staff: submissions and grades ----
  http.get(`${A}/assignments/:id/submissions`, ({ request, params }) => {
    const { item, course } = staffAssignment(String(params.id));
    const db = getDb();
    const rows = db.members.filter((m) => m.course_id === course.id && m.role === "student" && (!m.left_on || (submissionOf(item.id, m.user_id)?.attempts.length ?? 0) > 0)).map((m) => {
      const u = userById(m.user_id);
      const latest = submissionOf(item.id, m.user_id)?.attempts[0];
      return {
        student_id: u.id, name: u.name, surname: u.surname, status: latest ? "submitted" : "not_submitted", former: !!m.left_on,
        latest: latest ? { id: latest.id, attempt: latest.attempt, submitted_at: latest.submitted_at, is_late: latest.is_late } : null,
      };
    }).sort((a, b) => a.surname.localeCompare(b.surname) || a.name.localeCompare(b.name));
    return paged(rows, request);
  }),
  http.get(`${A}/assignments/:id/submissions/:sid`, ({ request, params }) => {
    const { item } = staffAssignment(String(params.id));
    return paged((submissionOf(item.id, String(params.sid))?.attempts ?? []).map((a) => ({ ...a, content: wireContent(item, a.content) })), request);
  }),
  http.put(`${A}/assignments/:id/grades/:sid`, async ({ request, params }) => {
    const { item, course, user } = staffAssignment(String(params.id));
    const m = memberOf(course.id, String(params.sid));
    if (!m || m.role !== "student") throw notFound();
    if (m.left_on) throw problem(409, "Conflict", "This student left the course; their grades are locked");
    const b = (await request.json()) as { score?: string | null; feedback?: string | null };
    const db = getDb();
    let g = gradeOf(item.id, m.user_id);
    if (!g) { g = { assignment_id: item.id, student_id: m.user_id, draft_score: null, draft_feedback: null, published_score: null, published_feedback: null, published_at: null, history: [] }; db.grades.push(g); }
    if ("score" in b) {
      const next = b.score === null || b.score === "" || b.score === undefined ? null : decimal(b.score, "score", item.assignment!.max_points);
      g.draft_score = next;
    }
    if ("feedback" in b) g.draft_feedback = b.feedback?.trim() ? b.feedback.trim() : null;
    g.history.unshift({ action: "save_draft", score: g.draft_score, feedback: g.draft_feedback, actor_id: user.id, at: MOCK_NOW.toISOString() });
    return HttpResponse.json({
      draft_score: g.draft_score, draft_feedback: g.draft_feedback, published_score: g.published_score, published_feedback: g.published_feedback, has_unpublished_changes: hasUnpublishedChanges(g),
    });
  }),
  http.post(`${A}/assignments/:id/grades/publish`, async ({ request, params }) => {
    const { item, course, user } = staffAssignment(String(params.id));
    const b = (await request.json()) as { students: "all" | string[] };
    const ids = new Set(b.students === "all" ? activeStudents(course.id).map((m) => m.user_id) : b.students);
    let published = 0;
    for (const g of getDb().grades.filter((x) => x.assignment_id === item.id && ids.has(x.student_id) && hasUnpublishedChanges(x))) {
      if (memberOf(course.id, g.student_id)?.left_on) continue;
      g.published_score = g.draft_score; g.published_feedback = g.draft_feedback;
      g.published_at = g.draft_score === null && g.draft_feedback === null ? null : MOCK_NOW.toISOString();
      g.history.unshift({ action: "publish", score: g.published_score, feedback: g.published_feedback, actor_id: user.id, at: MOCK_NOW.toISOString() });
      published++;
    }
    return HttpResponse.json({ published });
  }),
  http.get(`${A}/assignments/:id/grades/:sid/history`, ({ params }) => {
    const { item } = staffAssignment(String(params.id));
    const rows = gradeOf(item.id, String(params.sid))?.history ?? [];
    const items = rows.map((h, n) => {
      const a = h.actor_id ? userById(h.actor_id) : null;
      return { id: uid(`${item.id}:${params.sid}:${n}`), action: h.action, score: h.score, feedback: h.feedback, actor: a ? { user_id: a.id, name: a.name, surname: a.surname } : null, at: h.at };
    });
    return HttpResponse.json({ items, next_cursor: null });
  }),
  http.get(`${A}/courses/:id/gradebook`, ({ request, params }) => {
    const { course } = requireStaff(String(params.id));
    const db = getDb();
    const items = flatItems(course, true).filter((i) => i.assignment && toCents(i.assignment.max_points) > 0);
    const columns = items.map((i) => ({ assignment_id: i.id, title: i.title, max_points: i.assignment!.max_points, is_bonus: i.assignment!.is_bonus, status: i.status }));
    const rows = db.members.filter((m) => m.course_id === course.id && m.role === "student" && (!m.left_on || db.grades.some((g) => g.student_id === m.user_id && items.some((i) => i.id === g.assignment_id)))).map((m) => {
      const u = userById(m.user_id);
      const cells = items.flatMap((i) => {
        const g = gradeOf(i.id, m.user_id);
        return g ? [{ assignment_id: i.id, draft_score: g.draft_score, draft_feedback: g.draft_feedback, published_score: g.published_score, published_feedback: g.published_feedback, has_unpublished_changes: hasUnpublishedChanges(g) }] : [];
      });
      return { user_id: u.id, name: u.name, surname: u.surname, former: !!m.left_on, cells };
    }).sort((a, b) => a.surname.localeCompare(b.surname) || a.name.localeCompare(b.name));
    const url = new URL(request.url);
    const limit = Math.min(Number(url.searchParams.get("limit") ?? 50) || 50, 200);
    const start = Number(url.searchParams.get("cursor") ?? 0) || 0;
    return HttpResponse.json({ columns, items: rows.slice(start, start + limit), next_cursor: start + limit < rows.length ? String(start + limit) : null });
  }),
  http.get(`${A}/assignments/:id/responses`, ({ params }) => {
    const { item, course } = staffAssignment(String(params.id));
    if (item.assignment!.type !== "form") throw notFound();
    const students = activeStudents(course.id);
    const rows = students.flatMap((m) => {
      const a = submissionOf(item.id, m.user_id)?.attempts[0];
      if (!a) return [];
      const u = userById(m.user_id);
      return [{ student_id: u.id, name: u.name, surname: u.surname, attempt: a.attempt, submitted_at: a.submitted_at, is_late: a.is_late, answers: (wireContent(item, a.content) as { answers: Record<string, unknown> }).answers }];
    });
    const done = new Set(rows.map((r) => r.student_id));
    const missing = students.filter((m) => !done.has(m.user_id)).map((m) => ({ user_id: m.user_id, name: userById(m.user_id).name, surname: userById(m.user_id).surname }));
    return HttpResponse.json({ total_students: students.length, missing, rows: rows.sort((a, b) => b.submitted_at.localeCompare(a.submitted_at)) });
  }),

  // ---- teacher overview ----
  http.get(`${A}/me/teaching`, () => {
    const user = requireUser();
    const db = getDb();
    const courses = db.courses.filter((c) => c.kind === "regular" && c.status !== "archived" && memberOf(c.id, user.id) && ["teacher", "assistant"].includes(memberOf(c.id, user.id)!.role));
    const ids = courses.map((c) => c.id);
    const to_grade = toGradeList(ids);
    const unpublished: TeachingOverview["unpublished"] = [];
    const deadlines: TeachingOverview["deadlines"] = [];
    const content: TeachingOverview["unpublished_content"] = [];
    for (const c of courses) {
      const info = courseInfo(c);
      for (const item of db.items.filter((i) => i.course_id === c.id)) {
        if (item.assignment) {
          const students = activeStudents(c.id);
          const draftCount = students.filter((s) => { const g = gradeOf(item.id, s.user_id); return !!g && hasUnpublishedChanges(g); }).length;
          if (draftCount) unpublished.push({ assignment_id: item.id, assignment_title: item.title, course_id: c.id, course_code: info.code, course_title: c.title, draft_count: draftCount, auto_scored: item.assignment.type === "quiz" });
          const due = item.assignment.due_at;
          if (due && new Date(due) >= MOCK_NOW) {
            deadlines.push({
              assignment_id: item.id, course_id: c.id, course_code: info.code, course_title: c.title, title: item.title, due_at: due,
              submitted: students.filter((s) => (submissionOf(item.id, s.user_id)?.attempts.length ?? 0) > 0).length, total: students.length, not_open: item.status === "draft",
            });
          }
        }
        const parentDraft = db.modules.find((m) => m.id === item.module_id)?.status === "draft";
        if (item.status === "draft" && !parentDraft) content.push({ item_id: item.id, course_id: c.id, course_code: info.code, course_title: c.title, title: item.title, reason: "draft" });
        if (item.unpublished_changes) content.push({ item_id: item.id, course_id: c.id, course_code: info.code, course_title: c.title, title: item.title, reason: "edits" });
      }
    }
    const overview: TeachingOverview = { to_grade, unpublished, deadlines: deadlines.sort((a, b) => a.due_at.localeCompare(b.due_at)), unpublished_content: content };
    return HttpResponse.json(overview);
  }),
];

