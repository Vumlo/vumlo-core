import type { GradebookCell, SubmissionRow } from "./api/types";
import { toCents } from "./points";

export type CellState = "pub" | "draft" | "grade" | "none";
export type StatusFilter = "all" | CellState;
export type WorkspaceFilter = "grade" | "draft" | "all";

/** Where one student's work on one assignment stands: published, draft grade, waiting for a grade, or nothing. */
export function cellState(cell: GradebookCell | undefined, sub: SubmissionRow | undefined): CellState {
  if (cell && cell.has_unpublished_changes && cell.draft_score !== null && cell.draft_score !== undefined) return "draft";
  if (cell?.published_score) return "pub";
  if (sub?.status === "submitted" && !(cell && cell.draft_score)) return "grade";
  return "none";
}

/** A row passes when one single cell satisfies the status and the late filter together. */
export function matchesStatus(cells: { state: CellState; late: boolean }[], f: { status: StatusFilter; late: boolean }): boolean {
  if (f.status === "all" && !f.late) return true;
  return cells.some((c) => (f.status === "all" || c.state === f.status) && (!f.late || c.late));
}

export function inWorkspaceFilter(state: CellState, f: WorkspaceFilter): boolean {
  return f === "all" || state === f;
}

/** The nearest id in `visible` from `current` in the direction `delta`, or null at the end or for an unknown `current`. */
export function neighbour(order: string[], visible: ReadonlySet<string>, current: string, delta: 1 | -1): string | null {
  const from = order.indexOf(current);
  if (from < 0) return null;
  for (let i = from + delta; i >= 0 && i < order.length; i += delta) {
    const id = order[i];
    if (id !== undefined && visible.has(id)) return id;
  }
  return null;
}

/**
 * What the server's score of one quiz question lets staff conclude about the options the student picked, when the
 * attempt carries no answer key: a full score means every pick is right, and a zero on a single-choice question
 * means its one pick is wrong. Anything else (a partial score, a zero on a multiple-choice question) cannot be
 * decided from the score alone, so it returns null and the caller shows the score without marking the options.
 */
export function autoResult(kind: string, picks: string[], got: string, points: string): { correct: string[] } | null {
  if (toCents(got) === toCents(points)) return { correct: picks };
  if (kind === "single" && toCents(got) === 0) return { correct: [] };
  return null;
}
