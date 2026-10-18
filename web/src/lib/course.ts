import { dueLabel, relative, shortDay } from "./format";
import { translate } from "./i18n/core";
import type { MessageKey } from "./i18n/core";
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

const TYPE_LABELS: Record<string, MessageKey> = {
  text: "type.text",
  file: "type.file",
  quiz: "type.quiz",
  form: "type.form",
  no_submission: "type.inClass",
};

export const typeLabel = (type: string) => (TYPE_LABELS[type] ? translate(TYPE_LABELS[type]!) : type);

/** What the outline shows at the right of an assignment. */
export function outlineMark(meta: ItemMeta | undefined): { text: string; tone: "ok" | "accent" | "muted" | "danger" } {
  switch (meta?.status) {
    case "graded":
      return { text: meta.score ?? translate("status.graded"), tone: "ok" };
    case "draft":
      return { text: translate("common.draft"), tone: "accent" };
    case "submitted":
      return { text: translate("status.submitted"), tone: "ok" };
    case "missed":
      return { text: translate("status.missed"), tone: "danger" };
    case "in_class":
      return { text: translate("status.inClass"), tone: "muted" };
    default:
      return { text: translate("status.todo"), tone: "muted" };
  }
}

export type PillTone = "plain" | "strong" | "accent" | "ok" | "danger";

export function statusPill(status: WorkStatus): { label: string; tone: PillTone } {
  switch (status) {
    case "graded":
      return { label: translate("status.graded"), tone: "ok" };
    case "submitted":
      return { label: translate("status.waiting"), tone: "strong" };
    case "draft":
      return { label: translate("status.draftSaved"), tone: "accent" };
    case "missed":
      return { label: translate("status.notSubmitted"), tone: "danger" };
    case "in_class":
      return { label: translate("status.nothingToSubmit"), tone: "strong" };
    default:
      return { label: translate("status.notStarted"), tone: "plain" };
  }
}

/** "Closed 9 Oct" / "Due 23 Oct" for the course lists. */
export function dueShort(due: string | null, classOn: string | null | undefined, now: Date): string {
  if (classOn) return translate(new Date(classOn) < now ? "due.tookPlace" : "due.inClass", { day: shortDay(classOn) });
  if (!due) return translate("due.none");
  return translate(new Date(due) < now ? "due.closed" : "due.due", { day: shortDay(due) });
}

export function dueWithRelative(due: string): string {
  return `${dueLabel(due)}, ${relative(due)}`;
}
