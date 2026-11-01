import { HttpResponse } from "msw";
import { MOCK_NOW } from "@/lib/clock";
import { fromCents, sumPoints, toCents } from "@/lib/points";
import type { FormConfig, QuizConfig, QuizQuestion } from "@/lib/api/gaps";
import type { MyWorkItem, WorkStatus } from "@/lib/api/types";
import type { ContentModule, ContentTree, ItemSummary, Module } from "@/lib/api/types";
import { getDb, type DbCourse, type DbGrade, type DbItem, type DbMember, type DbSubmission, type DbUser } from "./db";

export const SESSION_KEY = "vumlo.mock.session";

export function problem(status: number, title: string, detail?: string, errors?: { field: string; message: string }[]): Response {
  return HttpResponse.json(
    { type: "about:blank", title, status, ...(detail ? { detail } : {}), ...(errors ? { errors } : {}) },
    { status, headers: { "Content-Type": "application/problem+json" } },
  );
}

export const notFound = () => problem(404, "Not found");
export const forbidden = () => problem(403, "Forbidden");

export function sessionUser(): DbUser | null {
  const id = localStorage.getItem(SESSION_KEY);
  const u = id ? getDb().users.find((x) => x.id === id) : undefined;
  return u && u.status === "active" ? u : null;
}

export function requireUser(): DbUser {
  const u = sessionUser();
  if (!u) throw problem(401, "Unauthorized");
  return u;
}

export function requireAdmin(): DbUser {
  const u = requireUser();
  if (u.role !== "admin") throw notFound();
  return u;
}

export const publicUser = ({ password: _password, last_sign_in: _last, ...u }: DbUser) => u;

export function userById(id: string): DbUser {
  const u = getDb().users.find((x) => x.id === id);
  if (!u) throw notFound();
  return u;
}

export const fullName = (u: Pick<DbUser, "name" | "surname">) => `${u.name} ${u.surname}`;
export const teacherName = (u: Pick<DbUser, "name" | "surname">) => `Dr. ${u.name} ${u.surname}`;

// ---- courses and access ----

export function courseById(id: string): DbCourse {
  const c = getDb().courses.find((x) => x.id === id);
  if (!c) throw notFound();
  return c;
}

export function memberOf(courseId: string, userId: string): DbMember | undefined {
  return getDb().members.find((m) => m.course_id === courseId && m.user_id === userId);
}

export function isStaff(user: DbUser, course: DbCourse): boolean {
  if (user.role === "admin") return true;
  const m = memberOf(course.id, user.id);
  return !!m && (m.role === "teacher" || m.role === "assistant") && !m.left_on;
}

/** Students see a course only once it is published (or archived, read only) and only when on its roster. */
export function canView(user: DbUser, course: DbCourse): boolean {
  if (isStaff(user, course)) return true;
  const m = memberOf(course.id, user.id);
  return !!m && course.status !== "draft";
}

export function requireView(courseId: string, user = requireUser()) {
  const course = courseById(courseId);
  if (!canView(user, course)) throw notFound();
  return { course, user, staff: isStaff(user, course) };
}

export function requireStaff(courseId: string, user = requireUser()) {
  const course = courseById(courseId);
  if (!canView(user, course)) throw notFound();
  if (!isStaff(user, course)) throw forbidden();
  return { course, user };
}

export const activeStudents = (courseId: string) => getDb().members.filter((m) => m.course_id === courseId && m.role === "student" && !m.left_on);

export function ownerOf(course: DbCourse): DbUser {
  return userById(course.owner_id);
}

export function termName(course: DbCourse): string | null {
  return getDb().terms.find((t) => t.id === course.term_id)?.name ?? null;
}

export function termEnded(course: DbCourse): boolean {
  const t = getDb().terms.find((x) => x.id === course.term_id);
  return !!t && new Date(`${t.ends_on}T23:59:59+03:00`) < MOCK_NOW;
}

// ---- content ----

export function moduleOf(item: DbItem): Module {
  const m = getDb().modules.find((x) => x.id === item.module_id);
  if (!m) throw notFound();
  return m;
}

export function itemById(id: string): DbItem {
  const i = getDb().items.find((x) => x.id === id);
  if (!i) throw notFound();
  return i;
}

/** The rule of ADR-0010: course, module and item published and publish_at reached. */
export function itemVisibleToStudent(item: DbItem, course: DbCourse): boolean {
  const m = moduleOf(item);
  const reached = (d: string | null | undefined) => !d || new Date(d) <= MOCK_NOW;
  return course.status !== "draft" && m.status === "published" && reached(m.publish_at) && item.status === "published" && reached(item.publish_at);
}

export const summary = (i: DbItem): ItemSummary => ({
  id: i.id, kind: i.kind, title: i.title, status: i.status, publish_at: i.publish_at, position: i.position,
  ...(i.assignment ? { assignment: stripConfig(i.assignment) } : {}),
});

function stripConfig(a: NonNullable<DbItem["assignment"]>) {
  const { config: _config, ...rest } = a;
  return rest;
}

export function contentTree(course: DbCourse, staff: boolean): ContentTree {
  const db = getDb();
  const visible = (i: DbItem) => staff || itemVisibleToStudent(i, course);
  const modOk = (m: Module) => staff || (m.status === "published" && (!m.publish_at || new Date(m.publish_at) <= MOCK_NOW));
  const tops = db.modules.filter((m) => m.course_id === course.id && !m.parent_id).sort((a, b) => a.position - b.position);
  const modules: ContentModule[] = tops.filter(modOk).map((m) => ({
    ...m,
    submodules: db.modules
      .filter((s) => s.parent_id === m.id && modOk(s))
      .sort((a, b) => a.position - b.position)
      .map((s) => ({ ...s, items: db.items.filter((i) => i.module_id === s.id && visible(i)).sort((a, b) => a.position - b.position).map(summary) })),
    items: db.items.filter((i) => i.module_id === m.id && visible(i)).sort((a, b) => a.position - b.position).map(summary),
  }));
  return { modules };
}

/** Every item the caller sees, in tree order. */
export function flatItems(course: DbCourse, staff: boolean): DbItem[] {
  const tree = contentTree(course, staff);
  const ids = tree.modules.flatMap((m) => [...m.items, ...m.submodules.flatMap((s) => s.items)].map((i) => i.id));
  return ids.map((id) => itemById(id));
}

// ---- submissions, grades, status ----

export function submissionOf(assignmentId: string, studentId: string): DbSubmission | undefined {
  return getDb().submissions.find((s) => s.assignment_id === assignmentId && s.student_id === studentId);
}

export function gradeOf(assignmentId: string, studentId: string): DbGrade | undefined {
  return getDb().grades.find((g) => g.assignment_id === assignmentId && g.student_id === studentId);
}

export const isPublished = (g: DbGrade | undefined) => !!g && g.published_at !== null && (g.published_score !== null || g.published_feedback !== null);

export function dueIn(item: DbItem): { open: boolean; closed: boolean } {
  const a = item.assignment;
  if (!a?.due_at) return { open: true, closed: false };
  const past = new Date(a.due_at) < MOCK_NOW;
  const closed = past && !(a.accept_late && (!a.late_until || new Date(a.late_until) > MOCK_NOW));
  return { open: !closed, closed };
}

export function workStatus(item: DbItem, studentId: string): WorkStatus {
  const sub = submissionOf(item.id, studentId);
  if (isPublished(gradeOf(item.id, studentId))) return "graded";
  if (item.assignment?.type === "no_submission") return "in_class";
  if (sub && sub.attempts.length > 0) return "submitted";
  if (sub?.draft) return "draft";
  return dueIn(item).closed ? "missed" : "not_started";
}

export function workItem(item: DbItem, studentId: string): MyWorkItem {
  const g = gradeOf(item.id, studentId);
  const pub = isPublished(g);
  const a = item.assignment!;
  return {
    assignment_id: item.id, course_id: item.course_id, title: item.title, type: a.type,
    due_at: a.due_at ?? null, class_on: item.class_on ?? null, max_points: a.max_points, status: workStatus(item, studentId),
    score: pub ? (g!.published_score ?? null) : null, published_at: pub ? g!.published_at : null,
    draft_saved_at: submissionOf(item.id, studentId)?.draft?.updated_at ?? null,
  };
}

export function hasUnpublishedChanges(g: DbGrade): boolean {
  return g.draft_score !== g.published_score || g.draft_feedback !== g.published_feedback;
}

// ---- quizzes ----

export const quizOf = (item: DbItem) => item.assignment?.config as unknown as QuizConfig;
export const formOf = (item: DbItem) => item.assignment?.config as unknown as FormConfig;

/** Students never receive the answer key (ADR-0023). */
export function studentConfig(item: DbItem): Record<string, unknown> {
  const a = item.assignment!;
  if (a.type === "quiz") {
    const q = quizOf(item);
    return {
      ...q,
      questions: q.questions.map(({ correct: _c, answer: _a, tolerance: _t, grading_notes: _g, ...rest }) => rest),
    } as unknown as Record<string, unknown>;
  }
  return a.config;
}

export function scoreQuestion(q: QuizQuestion, answer: string | string[] | undefined): string | null {
  const pts = toCents(q.points);
  if (answer === undefined || answer === "" || (Array.isArray(answer) && answer.length === 0)) return "0.00";
  if (q.type === "single") return answer === q.correct?.[0] ? q.points : "0.00";
  if (q.type === "multi") {
    const picked = Array.isArray(answer) ? answer : [answer];
    const right = (q.correct ?? []).length;
    const good = picked.filter((p) => q.correct?.includes(p)).length;
    const bad = picked.length - good;
    if (!q.partial_credit) return good === right && bad === 0 ? q.points : "0.00";
    return fromCents(Math.max(0, Math.round((pts * (good - bad)) / right)));
  }
  if (q.type === "number") {
    const diff = Math.abs(Number(String(answer).replace(",", ".")) - Number(q.answer));
    return diff <= Number(q.tolerance ?? 0) ? q.points : "0.00";
  }
  return null;
}

export function autoScore(item: DbItem, answers: Record<string, string | string[]>): string {
  const parts = quizOf(item).questions.map((q) => scoreQuestion(q, answers[q.id]) ?? "0.00");
  return sumPoints(parts);
}

// ---- paging ----

export function paged<T>(items: T[], request: Request) {
  const url = new URL(request.url);
  const limit = Math.min(Number(url.searchParams.get("limit") ?? 50) || 50, 200);
  const start = Number(url.searchParams.get("cursor") ?? 0) || 0;
  const slice = items.slice(start, start + limit);
  return HttpResponse.json({ items: slice, next_cursor: start + limit < items.length ? String(start + limit) : null });
}

export function decimal(input: unknown, field: string, max?: string): string {
  const s = typeof input === "string" ? input.trim() : "";
  if (!/^\d+(\.\d{1,2})?$/.test(s)) throw problem(422, "Validation failed", undefined, [{ field, message: "Use a number with at most two decimals, for example 7.5" }]);
  if (max && toCents(s) > toCents(max)) throw problem(422, "Validation failed", undefined, [{ field, message: `The score can be at most ${max}` }]);
  return fromCents(toCents(s));
}

// ---- the `file` assignment type: the server's rules for its config and its content (content/types/file.rs) ----

const bad = (field: string, message: string) => problem(422, "Unprocessable", undefined, [{ field, message }]);

const IMAGE_MIMES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

/** Where a document is stored: the editor profile, and for a student's own work whose upload may be named. */
export type DocPlace = { profile: "full" | "student" | "question" | "option"; userId?: string; assignmentId?: string };

/**
 * What the server refuses in the `image` and `file` blocks of a document (ADR-0025, ADR-0069): a `file` block in a profile without it,
 * a file that is unknown, swept or somebody else's, and an `image` block whose file is not a PNG, JPEG, GIF or WebP. `path` is
 * where the document sits in the request, so an error names `body.blocks[2].props.file_id`.
 */
export function documentErrors(doc: unknown, place: DocPlace, path: string): { field: string; message: string }[] {
  const blocks = (doc as { blocks?: unknown } | null | undefined)?.blocks;
  if (!Array.isArray(blocks)) return [];
  const stored = getDb().files;
  const errors: { field: string; message: string }[] = [];
  type Block = { type?: string; props?: { file_id?: string }; children?: Block[] };
  // The server checks every block, nested ones too (a list item or a quote may hold a picture).
  const walk = (list: Block[], prefix: string) => list.forEach((b, i) => {
    if (b?.type !== "image" && b?.type !== "file") return void (Array.isArray(b?.children) && walk(b.children, `${prefix}[${i}].children`));
    const at = `${prefix}[${i}]`;
    if (b.type === "file" && (place.profile === "question" || place.profile === "option")) return void errors.push({ field: `${at}.type`, message: "a file block is not allowed here" });
    const id = b.props?.file_id;
    const file = typeof id === "string" ? stored.get(id) : undefined;
    // A student's file is usable in that student's drafts and attempts of the assignment it was uploaded for, new draft after an earlier attempt included;
    // another student's file, another assignment's and an item's (no owner) are not.
    const own = place.assignmentId === undefined || (file?.owner === place.userId && file?.target === `assignment:${place.assignmentId}`);
    if (!file || !own) errors.push({ field: `${at}.props.file_id`, message: `the file ${String(id)} cannot be used here` });
    else if (b.type === "image" && !IMAGE_MIMES.has(file.mime)) errors.push({ field: `${at}.props.file_id`, message: "an image block needs a PNG, JPEG, GIF or WebP file" });
  });
  walk(blocks as Block[], `${path}.blocks`);
  return errors;
}

/** Throws the 422 the server answers with when a document has a block it refuses. */
export function checkDocument(doc: unknown, place: DocPlace, path: string) {
  const errors = documentErrors(doc, place, path);
  if (errors.length) throw problem(422, "Unprocessable", undefined, errors);
}

/** The config of a `file` assignment as the server reads it: only `max_files` (1 to 20, default 5) and `accept`. */
export function fileSettings(config: Record<string, unknown>): { max: number; accept: string[] } {
  for (const key of Object.keys(config)) if (key !== "max_files" && key !== "accept") throw bad("config", `unknown field \`${key}\`: the file type has only \`max_files\` and \`accept\``);
  const max = "max_files" in config ? config.max_files : 5;
  if (typeof max !== "number" || !Number.isInteger(max) || max < 1 || max > 20) throw bad("config", "`max_files` must be a whole number from 1 to 20");
  const accept = "accept" in config ? config.accept : [];
  if (!Array.isArray(accept)) throw bad("config", "`accept` must be a list of extensions");
  if (accept.length > 30) throw bad("config", "`accept` has at most 30 entries");
  accept.forEach((entry, i) => {
    if (typeof entry !== "string" || !/^\.[a-z0-9]{2,16}$/.test(entry)) throw bad("config", `\`accept[${i}]\` must be a dot and 2 to 16 lowercase letters or digits, like \`.pdf\``);
    if (accept.indexOf(entry) !== i) throw bad("config", `\`accept[${i}]\` is listed twice`);
  });
  return { max, accept: accept as string[] };
}

/**
 * What the server refuses in the content of a `file` draft (or attempt, with `submit`): other keys, too many or repeated files,
 * a file that is not this student's upload for this assignment, a name outside `accept`, and, on submit, no file at all.
 */
export function checkFileContent(item: DbItem, userId: string, content: unknown, submit: boolean): { files: string[]; comment?: unknown } {
  const { max, accept } = fileSettings(item.assignment!.config);
  if (typeof content !== "object" || content === null || Array.isArray(content)) throw bad("", "must be an object");
  const c = content as Record<string, unknown>;
  if (Object.keys(c).some((k) => k !== "files" && k !== "comment")) throw bad("", "has only `files` and `comment`");
  const files = "files" in c ? c.files : [];
  if (!Array.isArray(files)) throw bad("files", "must be a list of file ids");
  const errors: { field: string; message: string }[] = [];
  if (files.length > max) errors.push({ field: "files", message: "has more files than the assignment takes" });
  const stored = getDb().files;
  files.forEach((id, i) => {
    const file = typeof id === "string" ? stored.get(id) : undefined;
    if (typeof id !== "string") errors.push({ field: `files[${i}]`, message: "must be a file id" });
    else if (files.indexOf(id) !== i) errors.push({ field: `files[${i}]`, message: "is listed twice" });
    else if (!file || file.owner !== userId || file.target !== `assignment:${item.id}`) errors.push({ field: `files[${i}]`, message: "the file cannot be used here" });
    else if (accept.length > 0 && !accept.some((e) => file.name.toLowerCase().endsWith(e))) errors.push({ field: `files[${i}]`, message: "is not a file type this assignment takes" });
  });
  errors.push(...documentErrors(c.comment, { profile: "student", userId, assignmentId: item.id }, "content.comment"));
  if (errors.length) throw problem(422, "Unprocessable", undefined, errors);
  if (submit && files.length === 0) throw bad("files", "add at least one file before you submit");
  // A comment with nothing in it is not kept.
  const comment = c.comment as { blocks?: { type?: string; content?: { text?: string }[] }[] } | undefined;
  const blank = !comment || (comment.blocks ?? []).every((b) => b.type === "paragraph" && (b.content ?? []).every((r) => !(r.text ?? "").trim()));
  return { files: files as string[], ...(blank ? {} : { comment }) };
}
