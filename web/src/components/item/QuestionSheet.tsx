"use client";

import { useState } from "react";
import { BlockEditor } from "@/components/doc/BlockEditor";
import { BlockRenderer } from "@/components/doc/BlockRenderer";
import { Icon } from "@/components/Icon";
import type { Answer } from "@/lib/api/gaps";
import type { Document, Feature } from "@/lib/api/types";
import { useT } from "@/lib/i18n";
import s from "./QuestionSheet.module.css";

export type QKind = "single" | "multi" | "number" | "short" | "long" | "select" | "url" | "people" | "date";

export type Q = {
  id: string;
  kind: QKind;
  label: string;
  short: string;
  help?: string;
  required?: boolean;
  points?: string;
  options: { id: string; text: string }[];
  maxPeople?: number;
};

export type QResult = { got: string; points: string; correct: string[]; answer: string | null; feedback: string | null };

type Props = {
  questions: Q[];
  answers: Record<string, Answer>;
  onChange: (id: string, value: Answer) => void;
  readOnly: boolean;
  result?: Record<string, QResult>;
  errors?: Record<string, string>;
  /** Names for the people field: id to display name. */
  classmates?: { id: string; name: string }[];
  me?: { id: string; name: string };
  ownerInitials?: string;
  ownerName?: string;
  mode: "quiz" | "form";
};

const LONG_FEATURES: Feature[] = ["text_styles", "lists", "math"];
const isDoc = (v: Answer | undefined): v is Document => !!v && typeof v === "object" && !Array.isArray(v);
const asList = (v: Answer | undefined): string[] => (Array.isArray(v) ? v : typeof v === "string" && v ? [v] : []);

export function isAnswered(q: Q, v: Answer | undefined): boolean {
  if (q.kind === "long") return isDoc(v) ? v.blocks.some((b) => ("content" in b && (b.content ?? []).length > 0) || b.type === "code") : false;
  if (q.kind === "multi" || q.kind === "people") return asList(v).length > 0;
  return typeof v === "string" && v.trim() !== "";
}

function Option({ role, checked, readOnly, result, note, children, onClick }: { role: "radio" | "checkbox"; checked: boolean; readOnly: boolean; result?: "right" | "wrong" | "missed"; note?: { text: string; tone: "ok" | "bad" | "muted" }; children: React.ReactNode; onClick: () => void }) {
  return (
    <button type="button" role={role} aria-checked={checked} className={s.option} data-readonly={readOnly} data-result={result} onClick={() => { if (!readOnly) onClick(); }}>
      <span className={s.mark} data-shape={role === "radio" ? "radio" : "check"} aria-hidden="true">
        {(checked || result === "missed") && <Icon name={result === "wrong" ? "close" : role === "radio" && !result ? "check" : "check"} size={13} strokeWidth={3} style={{ opacity: checked || result === "missed" ? 1 : 0 }} />}
      </span>
      <span>{children}</span>
      {note && <span className={s.note} data-tone={note.tone}>{note.text}</span>}
    </button>
  );
}

/** Quiz questions and form fields as one list of cards: the student answers them, or reads them after grading. */
export function QuestionSheet({ questions, answers, onChange, readOnly, result, errors, classmates = [], me, ownerInitials, ownerName, mode }: Props) {
  const t = useT();
  const [adding, setAdding] = useState<string | null>(null);
  const nameOf = (id: string) => (id === me?.id ? t("sheet.you", { name: me.name }) : classmates.find((c) => c.id === id)?.name ?? t("sheet.classmate"));

  return (
    <div className="stack" style={{ gap: 16 }}>
      {questions.map((q, i) => {
        const v = answers[q.id];
        const res = result?.[q.id];
        const full = res && res.got === res.points;
        const tone = res ? (full ? "ok" : "partial") : undefined;
        const header = mode === "form" ? (q.required ? t("sheet.required") : t("sheet.optional")) : `${t("sheet.question", { n: i + 1 })}${q.kind === "multi" ? `, ${t("sheet.pickAll")}` : ""}`;
        const labelId = `q${i + 1}-h`;
        return (
          <section key={q.id} id={`q${i + 1}`} aria-labelledby={labelId} className={`card ${s.q}`}>
            <div className={s.qHead}>
              <p className={s.qLabel}>{header}</p>
              {mode === "quiz" && q.points && <span className={s.qPoints} data-tone={tone}>{res ? `${res.got} / ${res.points}` : t("row.points", { points: q.points })}</span>}
            </div>
            <h2 id={labelId} className={s.qText}>{q.label}</h2>

            {(q.kind === "single" || q.kind === "multi") && (
              <div role={q.kind === "multi" ? "group" : "radiogroup"} aria-labelledby={labelId} className={s.options}>
                {q.options.map((o) => {
                  const picked = asList(v).includes(o.id);
                  const correct = res?.correct.includes(o.id) ?? false;
                  const state = res ? (picked && correct ? "right" : picked ? "wrong" : correct ? "missed" : undefined) : undefined;
                  const note = state === "right" ? { text: t("sheet.yourCorrect"), tone: "ok" as const } : state === "wrong" ? { text: t("sheet.yourWrong"), tone: "bad" as const } : state === "missed" ? { text: t("sheet.correct"), tone: "ok" as const } : undefined;
                  return (
                    <Option
                      key={o.id}
                      role={q.kind === "multi" ? "checkbox" : "radio"}
                      checked={picked}
                      readOnly={readOnly}
                      result={state}
                      note={note}
                      onClick={() => onChange(q.id, q.kind === "multi" ? (picked ? asList(v).filter((x) => x !== o.id) : [...asList(v), o.id]) : o.id)}
                    >
                      {o.text}
                    </Option>
                  );
                })}
              </div>
            )}

            {q.kind === "number" && (
              <div className="field">
                <label htmlFor={`in-${q.id}`}>{t("sheet.number")}</label>
                <input id={`in-${q.id}`} className="input" style={{ maxWidth: 220, borderColor: res ? (full ? "var(--ok-fg)" : "var(--danger)") : undefined }} inputMode="decimal" value={typeof v === "string" ? v : ""} readOnly={readOnly} onChange={(e) => onChange(q.id, e.target.value)} />
              </div>
            )}
            {q.kind === "short" && (
              <div className="field">
                <label htmlFor={`in-${q.id}`} className="sr-only">{q.label}</label>
                <input id={`in-${q.id}`} className="input" placeholder={t("asg.yourAnswer")} value={typeof v === "string" ? v : ""} readOnly={readOnly} aria-invalid={!!errors?.[q.id]} onChange={(e) => onChange(q.id, e.target.value)} />
              </div>
            )}
            {q.kind === "date" && (
              <input aria-labelledby={labelId} type="date" className="input" style={{ maxWidth: 220 }} value={typeof v === "string" ? v : ""} readOnly={readOnly} onChange={(e) => onChange(q.id, e.target.value)} />
            )}
            {q.kind === "select" && (
              <select aria-labelledby={labelId} className="select" value={typeof v === "string" ? v : ""} disabled={readOnly} aria-invalid={!!errors?.[q.id]} onChange={(e) => onChange(q.id, e.target.value)}>
                <option value="">{t("sheet.chooseVariant")}</option>
                {q.options.map((o) => <option key={o.id} value={o.id}>{o.text}</option>)}
              </select>
            )}
            {q.kind === "url" && (
              <input aria-labelledby={labelId} type="url" className="input" placeholder="https://" value={typeof v === "string" ? v : ""} readOnly={readOnly} aria-invalid={!!errors?.[q.id]} aria-describedby={q.help ? `help-${q.id}` : undefined} onChange={(e) => onChange(q.id, e.target.value)} />
            )}
            {q.kind === "people" && (
              <div role="group" aria-labelledby={labelId} className={s.chips}>
                {asList(v).map((id) => (
                  <span key={id} className={s.chip}>
                    {nameOf(id)}
                    {!readOnly && id !== me?.id && (
                      <button type="button" aria-label={t("sheet.remove", { name: nameOf(id) })} onClick={() => onChange(q.id, asList(v).filter((x) => x !== id))}><Icon name="close" size={13} strokeWidth={2.4} /></button>
                    )}
                  </span>
                ))}
                {!readOnly && asList(v).length < (q.maxPeople ?? 3) && (
                  <span className={s.add}>
                    <button type="button" className="btn" style={{ minHeight: 36, borderStyle: "dashed" }} aria-haspopup="menu" aria-expanded={adding === q.id} onClick={() => setAdding(adding === q.id ? null : q.id)}>
                      <Icon name="plus" size={14} strokeWidth={2.2} />{t("sheet.addClassmate")}
                    </button>
                    {adding === q.id && (
                      <div role="menu" className="popover" style={{ top: 40, left: 0, minWidth: 220 }}>
                        {classmates.filter((c) => !asList(v).includes(c.id)).map((c) => (
                          <button key={c.id} type="button" role="menuitem" className="menu-item" onClick={() => { onChange(q.id, [...asList(v), c.id]); setAdding(null); }}>{c.name}</button>
                        ))}
                        {classmates.filter((c) => !asList(v).includes(c.id)).length === 0 && <p className="muted" style={{ padding: 8, fontSize: 14 }}>{t("sheet.nobodyElse")}</p>}
                      </div>
                    )}
                  </span>
                )}
              </div>
            )}
            {q.kind === "long" && (
              readOnly ? (
                <div className={s.editor} style={{ padding: "12px 16px" }}>
                  {isDoc(v) && v.blocks.length ? <BlockRenderer doc={v} size={15} /> : <span className="muted">{t("sheet.noAnswer")}</span>}
                </div>
              ) : (
                <div className={s.editor}>
                  <BlockEditor value={isDoc(v) ? v : { v: 1, blocks: [] }} onChange={(d) => onChange(q.id, d)} features={LONG_FEATURES} label={q.label} mode="bar" minHeight={120} placeholder={t("asg.placeholder")} />
                </div>
              )
            )}

            {q.help && <p id={`help-${q.id}`} className={s.help}>{q.help}</p>}
            {errors?.[q.id] && <p role="alert" className={s.err}>{errors[q.id]}</p>}
            {res?.feedback && (
              <div className={s.feedback}>
                <span aria-hidden="true" className={s.avatar}>{ownerInitials}</span>
                <span><strong>{ownerName}: </strong>{res.feedback}</span>
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

export { s as sheetStyles };
