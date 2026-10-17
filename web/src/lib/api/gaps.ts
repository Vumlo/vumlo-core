/**
 * Types for data the server does not return yet. Every type here has an entry in
 * web/API_GAPS.md and a mocked endpoint in src/mocks/. Delete a type when its gap is closed.
 */
import type { Course, CourseRole, Document, Role } from "./types";
import type { Points } from "../points";

export type MyRole = "owner" | CourseRole;

/** What the institution shows everywhere: name for the navbar, language, time zone. */
export type Institution = {
  name: string;
  short_name: string;
  accent: string;
  language: "en" | "uk";
  time_zone: string;
  /** Shown to teachers in course settings; the scale itself is set in the config file (ADR-0022). */
  grading_scale: string;
};

/** The server-side settings an admin can read but not change (config file). */
export type ServerSettingsView = {
  grading_scale: string;
  failed_logins: string;
  session_lifetime: string;
  largest_document: string;
};

export type InstitutionAdmin = Institution & {
  accents: { name: string; value: string }[];
  server: ServerSettingsView;
  changed_at: string | null;
  changed_by: string | null;
};

/** `GET /courses/{id}/info`: the header of every course screen. */
export type CourseInfo = {
  course_id: string;
  code: string;
  description: string;
  term_name: string | null;
  owner_name: string;
  owner_email: string;
  student_count: number;
  former_count: number;
  assistant_count: number;
  updated_at: string;
  settings_saved_at: string | null;
  total_points: Points;
  graded_items: number;
  /** Grades exist, so the course cannot be deleted (ADR-0021). */
  has_grades: boolean;
};

/** `GET /me/course-cards`: one card per course on Courses and on the home page. */
export type CourseCard = {
  course: Course;
  code: string;
  term_name: string | null;
  term_ended: boolean;
  owner_name: string;
  my_role: MyRole;
  student_count: number;
  assistant_count: number;
  /** Students only. */
  progress: { submitted: number; total: number; graded: number } | null;
  next: { assignment_id: string; title: string; due_at: string } | null;
  /** Staff only. */
  todo: { kind: "grade" | "draft" | "info"; text: string; due_at?: string }[];
};

export type Template = {
  course: Course;
  sections: number;
  lectures: number;
  assignments: number;
  updated_at: string;
};

export type WorkStatus = "not_started" | "draft" | "submitted" | "graded" | "missed" | "in_class";

/** `GET /me/work`: every assignment of the caller's courses with the caller's own progress. */
export type MyWorkItem = {
  assignment_id: string;
  item_id: string;
  course_id: string;
  title: string;
  type: string;
  due_at: string | null;
  /** The date a no_submission assignment takes place (the server keeps `due_at` null for it). */
  class_on: string | null;
  max_points: Points;
  status: WorkStatus;
  score: Points | null;
  published_at: string | null;
  draft_saved_at: string | null;
};

export type MyWork = {
  courses: { id: string; code: string; title: string; term_name: string | null; owner_name: string }[];
  items: MyWorkItem[];
};

/** `GET /courses/{id}/item-meta`: per-item facts the content tree lacks. */
export type ItemMeta = {
  read_minutes?: number;
  class_on?: string | null;
  edited?: boolean;
  /** Student view. */
  status?: WorkStatus;
  score?: Points | null;
  draft_saved_at?: string | null;
  published_at?: string | null;
  attempts_used?: number;
  /** Staff view. */
  submitted?: number;
  to_grade?: number;
  draft_grades?: number;
  graded_total?: number;
};

export type ItemMetaMap = { items: Record<string, ItemMeta> };

export type PersonRow = {
  user_id: string;
  name: string;
  surname: string;
  /** Staff only; students do not get other people's emails. */
  email: string | null;
  role: CourseRole;
  is_owner: boolean;
  source: "direct" | "group";
  /** Group or subgroup name for students from a group. */
  group: string | null;
  former: boolean;
  left_on: string | null;
};

/** A person a teacher may add to a course (API_GAPS.md, "People"). */
export type Candidate = { user_id: string; name: string; surname: string; email: string; role: "teacher" | "student" | "admin" };

export type LinkedGroupRow = { link_id: string; name: string; student_count: number; linked_on: string };

/** `GET /courses/{id}/people`: the roster with names (the members endpoint has ids only). */
export type CoursePeople = { people: PersonRow[]; groups: LinkedGroupRow[] };

export type TeachingOverview = {
  to_grade: {
    student_id: string;
    student_name: string;
    student_surname: string;
    assignment_id: string;
    assignment_title: string;
    course_id: string;
    course_code: string;
    course_title: string;
    submitted_at: string;
    attempt: number;
    max_attempts: number | null;
  }[];
  unpublished: {
    assignment_id: string;
    assignment_title: string;
    course_id: string;
    course_code: string;
    course_title: string;
    draft_count: number;
    auto_scored: boolean;
  }[];
  deadlines: {
    assignment_id: string;
    course_id: string;
    course_code: string;
    title: string;
    due_at: string;
    submitted: number;
    total: number;
    note: string | null;
  }[];
  unpublished_content: { item_id: string; course_id: string; course_code: string; title: string; note: "draft" | "edits" }[];
};

export type GradeHistory = { entries: { what: string; at: string }[] };

export type ItemRevision = { has_unpublished_changes: boolean; last_published_at: string | null; size_bytes: number };

// ---- quizzes and forms (assignment types the server does not have yet) ----

export type QuizQuestionType = "single" | "multi" | "number" | "short" | "long";

export type QuizQuestion = {
  id: string;
  type: QuizQuestionType;
  prompt: string;
  points: Points;
  options: { id: string; text: string }[];
  /** Staff only: never sent to a student before the grades are published (ADR-0023). */
  correct?: string[];
  answer?: string;
  tolerance?: string;
  partial_credit?: boolean;
  shuffle?: boolean;
  grading_notes?: string;
};

export type QuizConfig = {
  questions: QuizQuestion[];
  shuffle_questions: boolean;
  show_correct: boolean;
};

export type FormFieldType = "short" | "long" | "choice" | "people" | "link" | "number" | "date";

export type FormField = {
  id: string;
  type: FormFieldType;
  label: string;
  help: string;
  required: boolean;
  options: { id: string; text: string; taken_by: string | null }[];
  rules: Record<string, string | boolean | number>;
  has_answers: boolean;
};

export type FormConfig = { note: string; fields: FormField[] };

/** One answer: a text, a list of option or person ids, or a block document for a long text. */
export type Answer = string | string[] | Document;

/** A student's answers to a quiz or form: question or field id to value. */
export type AnswerMap = Record<string, Answer>;

export type QuizResult = {
  published_at: string;
  score: Points;
  max_points: Points;
  questions: { id: string; got: Points; points: Points; correct: string[]; answer: string | null; feedback: string | null }[];
};

export type FormResponses = {
  total_students: number;
  missing: string[];
  rows: { team: string; variant: string; members: string; repo: string; submitted_at: string; late: boolean }[];
};

// ---- admin ----

export type AdminUserSummary = { all: number; student: number; teacher: number; admin: number; disabled: number };

export type AdminUserFacts = { facts: { k: string; v: string }[]; role_note: string | null };

export type AdminGroupOverview = {
  group_id: string;
  student_count: number;
  course_count: number;
  members: { user_id: string; name: string; surname: string; email: string; since: string }[];
  linked_courses: { course_id: string; code: string; title: string; owner_name: string; linked_on: string }[];
};

export type AdminTermStat = { term_id: string; courses: number; read_only: boolean; current: boolean };

export type AdminCourseRow = {
  course_id: string;
  code: string;
  title: string;
  owner_id: string;
  owner_name: string;
  term_id: string | null;
  student_count: number;
  status: "draft" | "published" | "archived";
};

export type AuditKind = "accounts" | "owners" | "groups" | "teachers";

export type AuditEvent = { id: string; at: string; who: string; what: string; detail: string; kind: AuditKind };

export type AuthRole = Role;

export type { Document };
