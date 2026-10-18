/**
 * Types for data the server does not return yet. Every type here has an entry in
 * web/API_GAPS.md and a mocked endpoint in src/mocks/. Delete a type when its gap is closed.
 */
import type { Course, CourseRole, Document, Role } from "./types";
import type { Points } from "../points";


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

export type AuthRole = Role;

export type { Document };

/** `POST /auth/google/start`: where to send the browser for Google's consent screen (ADR-0017, path 3). */
export type GoogleStart = { authorization_url: string };

/** Mock only: the mail a real server would send. Lets `pnpm dev:mock` follow a verification or reset link. */
export type MockMail = { id: string; to: string; kind: "verify" | "reset"; path: string; sent_at: string };
