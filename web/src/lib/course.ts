import { dueLabel, relative, shortDay } from "./format";
import type { ItemMeta, WorkStatus } from "./api/gaps";
import type { ContentModule, ContentSubModule, ContentTree, ItemSummary } from "./api/types";

export type Section = (ContentModule | ContentSubModule) & { depth: 0 | 1 };

export const isAssignment = (i: ItemSummary) => i.kind === "assignment";
export const kindOf = (i: Pick<ItemSummary, "kind">): "lecture" | "assignment" => (i.kind === "assignment" ? "assignment" : "lecture");

/** The sections of a tree in reading order; sub-modules come first inside their module (ADR-0020). */
export function sections(tree: ContentTree): Section[] {
  return tree.modules.flatMap((m): Section[] => [
    ...m.submodules.map((s): Section => ({ ...s, depth: 1 })),
    { ...m, depth: 0 },
  ]);
}

export function allItems(tree: ContentTree): ItemSummary[] {
  return tree.modules.flatMap((m) => [...m.submodules.flatMap((s) => s.items), ...m.items]);
}

export function findItem(tree: ContentTree, id: string): { item: ItemSummary; module: ContentModule | ContentSubModule; top: ContentModule } | undefined {
  for (const top of tree.modules) {
    for (const sub of top.submodules) {
      const item = sub.items.find((i) => i.id === id);
      if (item) return { item, module: sub, top };
    }
    const item = top.items.find((i) => i.id === id);
    if (item) return { item, module: top, top };
  }
  return undefined;
}

const TYPE_LABELS: Record<string, string> = {
  text: "Text answer",
  file: "File upload",
  quiz: "Quiz",
  form: "Form",
  no_submission: "Done in class",
};

export const typeLabel = (type: string) => TYPE_LABELS[type] ?? type;

/** What the outline shows at the right of an assignment. */
export function outlineMark(meta: ItemMeta | undefined): { text: string; tone: "ok" | "accent" | "muted" | "danger" } {
  switch (meta?.status) {
    case "graded":
      return { text: meta.score ?? "Graded", tone: "ok" };
    case "draft":
      return { text: "Draft", tone: "accent" };
    case "submitted":
      return { text: "Submitted", tone: "ok" };
    case "missed":
      return { text: "Missed", tone: "danger" };
    case "in_class":
      return { text: "In class", tone: "muted" };
    default:
      return { text: "To do", tone: "muted" };
  }
}

export type PillTone = "plain" | "strong" | "accent" | "ok" | "danger";

export function statusPill(status: WorkStatus): { label: string; tone: PillTone } {
  switch (status) {
    case "graded":
      return { label: "Graded", tone: "ok" };
    case "submitted":
      return { label: "Waiting for grade", tone: "strong" };
    case "draft":
      return { label: "Draft saved", tone: "accent" };
    case "missed":
      return { label: "Not submitted", tone: "danger" };
    case "in_class":
      return { label: "Nothing to submit", tone: "strong" };
    default:
      return { label: "Not started", tone: "plain" };
  }
}

/** "Closed 9 Oct" / "Due 23 Oct" for the course lists. */
export function dueShort(due: string | null, classOn: string | null | undefined, now: Date): string {
  if (classOn) return `${new Date(classOn) < now ? "Took place" : "In class"} ${shortDay(classOn)}`;
  if (!due) return "No deadline";
  return `${new Date(due) < now ? "Closed" : "Due"} ${shortDay(due)}`;
}

export function dueWithRelative(due: string): string {
  return `${dueLabel(due)}, ${relative(due)}`;
}
