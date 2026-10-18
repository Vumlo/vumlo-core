/**
 * Quizzes and forms between the screens and the server (ADR-0050, ADR-0051).
 *
 * The screens work with plain strings (a prompt, an option's text) and with one answer shape per
 * input; the server stores block documents and typed rules. These functions are the only place that
 * knows both shapes.
 */
import type { Answer, AnswerMap, FormConfig, FormField, FormResponses, QuizConfig, QuizQuestion, QuizQuestionType, QuizResult } from "./gaps";
import type { Points } from "../points";
import type { Document } from "./types";

type Json = Record<string, unknown>;

const QUIZ_TYPES: Record<QuizQuestionType, string> = { single: "single_choice", multi: "multiple_choice", number: "number", short: "short_text", long: "long_text" };
const QUIZ_TYPES_BACK = Object.fromEntries(Object.entries(QUIZ_TYPES).map(([ui, wire]) => [wire, ui])) as Record<string, QuizQuestionType>;

/** One paragraph per line. */
export function textDoc(text: string): Document {
  const blocks = text.split("\n").map((line, i) => ({ id: `b${i}`, type: "paragraph" as const, content: line ? [{ type: "text" as const, text: line }] : [] }));
  return { v: 1, attachments: [], blocks } as unknown as Document;
}

/** The text of a document: the text runs of its blocks, a line per block. */
export function docText(doc: unknown): string {
  const blocks = (doc as { blocks?: { content?: { text?: string }[] }[] } | null)?.blocks ?? [];
  return blocks.map((b) => (b.content ?? []).map((r) => r.text ?? "").join("")).join("\n");
}

function quizQuestionToWire(q: QuizQuestion): Json {
  const out: Json = { id: q.id, type: QUIZ_TYPES[q.type], points: q.points, prompt: textDoc(q.prompt) };
  if (q.grading_notes) out.grading_notes = q.grading_notes;
  if (q.type === "single" || q.type === "multi") {
    out.options = q.options.map((o) => ({ id: o.id, content: textDoc(o.text) }));
    out.correct = q.correct ?? [];
    out.shuffle_options = !!q.shuffle;
    if (q.type === "multi") out.partial_credit = !!q.partial_credit;
  } else if (q.type === "number") {
    out.answer = q.answer ?? "";
    out.tolerance = q.tolerance || "0";
  } else if (q.type === "short") {
    out.accepted = [q.answer ?? ""];
  }
  return out;
}

function quizQuestionFromWire(w: Json): QuizQuestion {
  const type = QUIZ_TYPES_BACK[String(w.type)] ?? "short";
  const options = ((w.options as { id: string; content: unknown }[] | undefined) ?? []).map((o) => ({ id: o.id, text: docText(o.content) }));
  const accepted = (w.accepted as string[] | undefined) ?? [];
  return {
    id: String(w.id),
    type,
    prompt: docText(w.prompt),
    points: String(w.points) as Points,
    options,
    correct: w.correct as string[] | undefined,
    answer: type === "short" ? accepted[0] : (w.answer as string | undefined),
    tolerance: w.tolerance as string | undefined,
    partial_credit: w.partial_credit as boolean | undefined,
    shuffle: w.shuffle_options as boolean | undefined,
    grading_notes: w.grading_notes as string | undefined,
  };
}

const FIELD_RULES: Record<string, string[]> = { short: ["max_length"], long: ["max_length"], choice: ["exclusive", "multiple"], people: ["min", "max"], number: ["min", "max"], link: [], date: [] };

/** The screens keep a team size as text such as "2-4"; the server has `min` and `max`. */
function rulesToWire(f: FormField): Json {
  const out: Json = {};
  for (const key of FIELD_RULES[f.type] ?? []) {
    const raw = f.rules[key];
    if (raw === undefined || raw === "" || raw === false) continue;
    if (key === "max_length") out[key] = Number(raw);
    else if (key === "exclusive" || key === "multiple") out[key] = raw === true || raw === "true";
    else if (f.type === "people") out[key] = Number(raw);
    else out[key] = String(raw);
  }
  if (f.type === "people" && f.rules.team_size) {
    const n = String(f.rules.team_size).match(/\d+/g)?.map(Number) ?? [];
    if (n.length) { out.min = n.length > 1 ? n[0] : 0; out.max = n[n.length - 1]; }
  }
  return out;
}

function formFieldToWire(f: FormField): Json {
  const out: Json = { id: f.id, type: f.type, label: f.label, help: f.help ?? "", required: f.required, rules: rulesToWire(f) };
  if (f.type === "choice") out.options = f.options.map((o) => ({ id: o.id, text: o.text }));
  return out;
}

function formFieldFromWire(w: Json): FormField {
  const rules = { ...((w.rules as Record<string, string | boolean | number> | undefined) ?? {}) };
  if (w.type === "people" && rules.max !== undefined) rules.team_size = `${rules.min ?? 1}-${rules.max}`;
  const options = ((w.options as { id: string; text: string; taken?: boolean; taken_by?: string | null }[] | undefined) ?? []).map((o) => ({
    id: o.id,
    text: o.text,
    taken_by: o.taken_by ?? (o.taken ? "·" : null),
  }));
  return { id: String(w.id), type: w.type as FormField["type"], label: String(w.label), help: String(w.help ?? ""), required: !!w.required, options, rules, has_answers: !!w.has_answers };
}

/** The `config` a screen edits, from what the server sent. Types without screens pass through. */
export function configFromWire(type: string, config: Json): Json {
  if (type === "quiz" && Array.isArray(config.questions)) {
    return { questions: (config.questions as Json[]).map(quizQuestionFromWire), shuffle_questions: !!config.shuffle_questions, show_correct: config.show_correct !== false } satisfies QuizConfig;
  }
  if (type === "form" && Array.isArray(config.fields)) {
    return { note: String(config.note ?? ""), fields: (config.fields as Json[]).map(formFieldFromWire) } satisfies FormConfig;
  }
  return config;
}

/** The `config` the server takes, from what a screen edited. */
export function configToWire(type: string, config: Json): Json {
  if (type === "quiz" && Array.isArray(config.questions)) {
    return { v: 1, questions: (config.questions as QuizQuestion[]).map(quizQuestionToWire), shuffle_questions: !!config.shuffle_questions, show_correct: config.show_correct !== false };
  }
  if (type === "form" && Array.isArray(config.fields)) {
    return { v: 1, note: String(config.note ?? ""), fields: (config.fields as FormField[]).map(formFieldToWire) };
  }
  return config;
}

/** An item as the screens read it: the `config` of a quiz or a form in the screens' shape. */
export function itemFromWire<T extends { assignment?: { type: string; config: Json } | null }>(item: T): T {
  const a = item.assignment;
  return a ? { ...item, assignment: { ...a, config: configFromWire(a.type, a.config) } } : item;
}

/** The server's answers for a quiz or form (lists of ids, a document for a long text). */
export function answersToWire(type: string, config: Json, answers: AnswerMap): AnswerMap {
  if (type !== "quiz") return answers;
  const kinds = new Map((config as unknown as QuizConfig).questions.map((q) => [q.id, q.type]));
  const out: AnswerMap = {};
  for (const [id, value] of Object.entries(answers)) {
    const kind = kinds.get(id);
    if (kind === "single") out[id] = typeof value === "string" ? (value ? [value] : []) : value;
    else if (kind === "long") out[id] = typeof value === "string" ? (value.trim() ? textDoc(value) : "") : value;
    else out[id] = value;
  }
  return out;
}

/** The screens' answers from the server's content. */
export function answersFromWire(type: string, config: Json, content: unknown): AnswerMap {
  const answers = ((content as { answers?: AnswerMap } | undefined)?.answers) ?? {};
  if (type !== "quiz") return answers;
  const kinds = new Map((config as unknown as QuizConfig).questions.map((q) => [q.id, q.type]));
  const out: AnswerMap = {};
  for (const [id, value] of Object.entries(answers)) {
    const kind = kinds.get(id);
    if (kind === "single" && Array.isArray(value)) out[id] = value[0] ?? "";
    else if (kind === "long" && typeof value === "object" && !Array.isArray(value)) out[id] = docText(value);
    else out[id] = value as Answer;
  }
  return out;
}

type WireResult = {
  score: string;
  max_points: string;
  published_at: string;
  feedback?: string | null;
  questions: { id: string; points: string; got?: string | null; answer?: unknown; correct?: unknown }[];
};

/** A quiz result for the sheet. The server's `correct` is a list of option ids, a number or accepted texts. */
export function quizResultFromWire(w: WireResult, config: QuizConfig): QuizResult {
  const kinds = new Map(config.questions.map((q) => [q.id, q.type]));
  return {
    published_at: w.published_at,
    score: w.score as Points,
    max_points: w.max_points as Points,
    questions: w.questions.map((q) => {
      const kind = kinds.get(q.id);
      const correct = kind === "single" || kind === "multi" ? ((q.correct as string[] | undefined) ?? []) : [];
      const typed = kind === "number" ? (q.correct as string | undefined) : kind === "short" ? (q.correct as string[] | undefined)?.[0] : undefined;
      return { id: q.id, got: (q.got ?? "0.00") as Points, points: q.points as Points, correct, answer: typed ?? null, feedback: null };
    }),
  };
}

type WireResponses = {
  total_students: number;
  missing: { name: string; surname: string }[];
  rows: { student_id: string; name: string; surname: string; submitted_at: string; is_late: boolean; answers: Record<string, unknown> }[];
};

/**
 * The responses table has fixed columns: the first short field is the team, the first choice the
 * variant, the people field the members and the first link the repository.
 */
export function formResponsesFromWire(w: WireResponses, config: FormConfig): FormResponses {
  const first = (type: string) => config.fields.find((f) => f.type === type);
  const names = new Map(w.rows.map((r) => [r.student_id, `${r.name} ${r.surname}`]));
  const text = (v: unknown) => (typeof v === "string" ? v : "");
  const choice = first("choice");
  return {
    total_students: w.total_students,
    missing: w.missing.map((m) => `${m.name} ${m.surname}`),
    rows: w.rows.map((r) => {
      const picked = (r.answers[choice?.id ?? ""] as string[] | undefined) ?? [];
      const members = ((r.answers[first("people")?.id ?? ""] as string[] | undefined) ?? []).map((id) => names.get(id) ?? "").filter(Boolean);
      return {
        team: text(r.answers[first("short")?.id ?? ""]) || `${r.name} ${r.surname}`,
        variant: choice?.options.filter((o) => picked.includes(o.id)).map((o) => o.text).join(", ") ?? "",
        members: [`${r.name} ${r.surname}`, ...members].join(", "),
        repo: text(r.answers[first("link")?.id ?? ""]).replace(/^https?:\/\//, ""),
        submitted_at: r.submitted_at,
        late: r.is_late,
      };
    }),
  };
}
