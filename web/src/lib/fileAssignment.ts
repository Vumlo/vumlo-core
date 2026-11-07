import type { Document } from "@/lib/api/types";
import type { MessageKey } from "@/lib/i18n/core";

/** The settings of a `file` assignment (spec 8): the server's defaults and limits. */
export const DEFAULT_MAX_FILES = 5;
export const MAX_FILES_LIMIT = 20;
const MAX_ACCEPT = 30;

export type FileConfig = { max_files: number; accept: string[] };

/** The config of a `file` assignment with the server's defaults filled in; `accept: []` takes any file. */
export function fileConfigOf(config: Record<string, unknown> | undefined): FileConfig {
  const max = config?.max_files;
  const accept = config?.accept;
  return {
    max_files: typeof max === "number" ? max : DEFAULT_MAX_FILES,
    accept: Array.isArray(accept) ? accept.filter((e): e is string => typeof e === "string") : [],
  };
}

/** The content of a `file` draft or attempt: file ids in order and an optional comment (a document of the `student` profile). */
export type FileContent = { files?: string[]; comment?: Document };
export const fileContentOf = (content: unknown): FileContent => (content ?? {}) as FileContent;

/** What `PUT .../draft` takes: no `comment` key while the comment is empty. */
export function fileContentToWire(files: string[], comment: Document | null): FileContent {
  return comment && !isBlankDoc(comment) ? { files, comment } : { files };
}

/** True when a file called `name` may be handed in: its last extension, compared without case, is in `accept` (empty takes any). */
export function acceptsName(accept: string[], name: string): boolean {
  if (accept.length === 0) return true;
  const dot = name.toLowerCase().lastIndexOf(".");
  return dot >= 0 && dot < name.length - 1 && accept.includes(name.toLowerCase().slice(dot));
}

/** The menu of the assignment settings: kinds of file with the extensions each chip stands for (`.jpg` also means `.jpeg`). */
export const ACCEPT_GROUPS: { title: MessageKey; types: { label: string; exts: string[] }[] }[] = [
  { title: "as.acceptGroup.documents", types: [".pdf", ".docx", ".doc", ".odt", ".rtf", ".txt", ".md"].map(one) },
  { title: "as.acceptGroup.sheets", types: [".xlsx", ".xls", ".ods", ".csv", ".tsv"].map(one) },
  { title: "as.acceptGroup.slides", types: [".pptx", ".ppt", ".odp"].map(one) },
  { title: "as.acceptGroup.images", types: [{ label: ".jpg", exts: [".jpg", ".jpeg"] }, ...[".png", ".gif", ".webp", ".heic", ".svg"].map(one)] },
  { title: "as.acceptGroup.audio", types: [".mp3", ".wav", ".m4a", ".ogg"].map(one) },
  { title: "as.acceptGroup.video", types: [".mp4", ".mov", ".webm"].map(one) },
  { title: "as.acceptGroup.code", types: [".py", ".js", ".java", ".c", ".h", ".cpp", ".hpp", ".html", ".css", ".sql", ".json", ".ipynb"].map(one) },
  { title: "as.acceptGroup.archives", types: [".zip"].map(one) },
];
function one(ext: string) {
  return { label: ext, exts: [ext] };
}
/** Every extension of the menu in menu order. */
export const MENU_EXTS = ACCEPT_GROUPS.flatMap((g) => g.types.flatMap((t) => t.exts));
const MENU = new Set(MENU_EXTS);

/** Splits a stored list into what the menu shows and what only the "other types" field can show. */
export function splitAccept(accept: string[]): { menu: string[]; other: string[] } {
  return { menu: accept.filter((e) => MENU.has(e)), other: accept.filter((e) => !MENU.has(e)) };
}

/** "pdf, DOCX" -> [".pdf", ".docx"]: lowercase, a leading dot, no empty entries. */
export function parseAccept(text: string): string[] {
  return text.split(/[\s,;]+/).filter(Boolean).map((e) => (e.startsWith(".") ? e : `.${e}`).toLowerCase());
}

export type AcceptProblem = { kind: "bad" | "twice"; entry: string } | { kind: "many" };

/** The first rule of the server that `accept` breaks: a dot and 1 to 16 letters or digits, no repeats, at most 30. */
export function acceptProblem(accept: string[]): AcceptProblem | null {
  if (accept.length > MAX_ACCEPT) return { kind: "many" };
  const seen = new Set<string>();
  for (const entry of accept) {
    if (!/^\.[a-z0-9]{1,16}$/.test(entry)) return { kind: "bad", entry };
    if (seen.has(entry)) return { kind: "twice", entry };
    seen.add(entry);
  }
  return null;
}

/** True for a whole number from 1 to 20. */
export const validMaxFiles = (text: string): boolean => /^\d+$/.test(text.trim()) && Number(text) >= 1 && Number(text) <= MAX_FILES_LIMIT;

/** True when a comment has nothing a reader could see. Conservative: any block but an empty paragraph counts as content. */
export function isBlankDoc(doc: Document): boolean {
  return doc.blocks.every((b) => b.type === "paragraph" && !(("children" in b ? b.children : undefined) as unknown[] | undefined)?.length && (b.content ?? []).every((r) => r.type === "text" && r.text.trim() === ""));
}
