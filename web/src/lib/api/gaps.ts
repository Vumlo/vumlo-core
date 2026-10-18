/**
 * Types for data the server does not return yet. Every type here has an entry in
 * web/API_GAPS.md and a mocked endpoint in src/mocks/. Delete a type when its gap is closed.
 */
import type { Course, CourseRole, Document, Role } from "./types";
import type { Points } from "../points";

export type MyRole = "owner" | CourseRole;

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

/** `POST /auth/google/start`: where to send the browser for Google's consent screen (ADR-0017, path 3). */
export type GoogleStart = { authorization_url: string };

/** Mock only: the mail a real server would send. Lets `pnpm dev:mock` follow a verification or reset link. */
export type MockMail = { id: string; to: string; kind: "verify" | "reset"; path: string; sent_at: string };

/** One hit of `GET /search`: a course, or a lecture or assignment inside one. */
export type SearchHit = {
  kind: "course" | "lecture" | "assignment";
  id: string;
  title: string;
  course_id: string;
  course_title: string;
  course_code: string;
  /** A short piece of text around the match, or null for a course. */
  snippet: string | null;
  /** For an assignment, when it is due; null otherwise. */
  due_at: string | null;
  /** Draft items are only returned to staff. */
  status: "draft" | "published";
};
