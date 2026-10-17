import { now } from "./clock";
import type { Assignment } from "./api/types";

export type Deadline = { due: Date | null; open: boolean; closed: boolean; late: boolean };

/** Whether work is still taken: before `due_at`, or inside the late window (ADR-0035). */
export function deadline(a: Pick<Assignment, "due_at" | "accept_late" | "late_until">, at: Date = now()): Deadline {
  if (!a.due_at) return { due: null, open: true, closed: false, late: false };
  const due = new Date(a.due_at);
  if (at <= due) return { due, open: true, closed: false, late: false };
  const lateOk = !!a.accept_late && (!a.late_until || at <= new Date(a.late_until));
  return { due, open: lateOk, closed: !lateOk, late: lateOk };
}
