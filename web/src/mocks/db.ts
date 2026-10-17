import { MOCK_NOW } from "@/lib/clock";
import type { Attempt, Course, CourseGroupLink, Group, Item, Membership, Module, Session, Subgroup, SubmissionRow, Term, User } from "@/lib/api/types";
import type { AuditEvent, Institution, InstitutionAdmin } from "@/lib/api/gaps";
import type { Document } from "@/lib/api/types";

export type DbUser = User & { password: string; last_sign_in: string | null };
export type DbCourse = Course & { code: string; description: string; settings_saved_at: string | null; template_updated_at?: string };
export type DbMember = Membership & { group_label: string | null; left_on: string | null };
export type DbItem = Item & {
  read_minutes?: number;
  class_on?: string | null;
  last_published_at: string | null;
  unpublished_changes: boolean;
  /** What students keep seeing while a published item has unpublished changes. */
  published_body?: Item["body"];
  published_title?: string;
};
export type DbSubmission = {
  assignment_id: string;
  student_id: string;
  draft: { content: unknown; updated_at: string } | null;
  attempts: (Omit<Attempt, "content"> & { content: unknown })[];
};
export type DbGrade = {
  assignment_id: string;
  student_id: string;
  draft_score: string | null;
  draft_feedback: string | null;
  published_score: string | null;
  published_feedback: string | null;
  published_at: string | null;
  history: { what: string; at: string }[];
};
export type DbGroupMember = { group_id: string; subgroup_id: string | null; user_id: string; since: string };
export type DbLink = CourseGroupLink & { linked_on: string };

export type Db = {
  users: DbUser[];
  terms: Term[];
  groups: Group[];
  subgroups: Subgroup[];
  groupMembers: DbGroupMember[];
  courses: DbCourse[];
  members: DbMember[];
  links: DbLink[];
  modules: Module[];
  items: DbItem[];
  submissions: DbSubmission[];
  grades: DbGrade[];
  sessions: Session[];
  audit: AuditEvent[];
  institution: InstitutionAdmin;
  formResponses: Record<string, { team: string; variant: string; members: string; repo: string; submitted_at: string; late: boolean }[]>;
  quizResults: Record<string, Record<string, { got: string; feedback: string | null }>>;
};

export const NOW_ISO = MOCK_NOW.toISOString();

/** A stable uuid-shaped id from a readable key, so links and tests are repeatable. */
export function uid(key: string): string {
  let h1 = 0x811c9dc5;
  let h2 = 0x01000193;
  for (let i = 0; i < key.length; i++) {
    h1 = Math.imul(h1 ^ key.charCodeAt(i), 0x01000193) >>> 0;
    h2 = Math.imul(h2 + key.charCodeAt(i), 0x85ebca6b) >>> 0;
  }
  const hex = (n: number) => n.toString(16).padStart(8, "0");
  const a = hex(h1);
  const b = hex(h2);
  const c = hex(Math.imul(h1 ^ h2, 0xc2b2ae35) >>> 0);
  const d = hex(Math.imul(h2 + h1, 0x27d4eb2f) >>> 0);
  return `${a}-${b.slice(0, 4)}-7${b.slice(5, 8)}-8${c.slice(1, 4)}-${c.slice(4)}${d.slice(0, 4)}`;
}

const ZONE = "Europe/Kyiv";

function offsetMs(instant: Date): number {
  const p: Record<string, number> = {};
  for (const x of new Intl.DateTimeFormat("en-GB", { timeZone: ZONE, year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric", hourCycle: "h23" }).formatToParts(instant)) {
    if (x.type !== "literal") p[x.type] = Number(x.value);
  }
  return Date.UTC(p.year!, p.month! - 1, p.day!, p.hour!, p.minute!, p.second!) - Math.floor(instant.getTime() / 1000) * 1000;
}

/** An ISO instant from local institution time (Europe/Kyiv, with its daylight saving): at("2026-10-23 23:59"). */
export function at(local: string): string {
  const wall = new Date(`${local.replace(" ", "T")}:00Z`);
  let t = wall.getTime() - offsetMs(wall);
  t = wall.getTime() - offsetMs(new Date(t));
  return new Date(t).toISOString();
}

export function institutionPublic(i: InstitutionAdmin): Institution {
  return { name: i.name, short_name: i.short_name, accent: i.accent, language: i.language, time_zone: i.time_zone, grading_scale: i.server.grading_scale };
}

export type { Document, SubmissionRow };

let current: Db | null = null;

export function getDb(): Db {
  if (!current) throw new Error("mock database used before it was created");
  return current;
}

export function setDb(db: Db) {
  current = db;
}
