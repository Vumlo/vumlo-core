"use client";

import { useState } from "react";
import { BlockRenderer } from "@/components/doc/BlockRenderer";
import { formQuestions, quizQuestions } from "@/components/item/SheetPage";
import { QuestionSheet, type QResult } from "@/components/item/QuestionSheet";
import type { Answer, AnswerMap, FormConfig, QuizConfig } from "@/lib/api/gaps";
import { answersFromWire } from "@/lib/api/sheetWire";
import type { Attempt, Document, Item } from "@/lib/api/types";
import { weekdayDate, timeOf } from "@/lib/format";
import { useT } from "@/lib/i18n";
import { toCents } from "@/lib/points";
import s from "./GradeWorkspace.module.css";

const isDoc = (v: unknown): v is Document => !!v && typeof v === "object" && !Array.isArray(v) && "blocks" in v;

/** What one student handed in, read-only: a text as a document, a quiz or a form as its questions with the answers. */
export function SubmissionView({ item, attempts }: { item: Item; attempts: Attempt[]; studentName: string }) {
  const t = useT();
  const newestFirst = [...attempts].sort((a, b) => b.attempt - a.attempt);
  const [picked, setPicked] = useState<number | null>(null);
  const shown = newestFirst.find((a) => a.attempt === picked) ?? newestFirst[0];
  if (!shown) return <p className="muted">{t("gb.nothing")}</p>;

  return (
    <div>
      <p className={s.how}>
        <span>{t("gb.submittedAt", { day: weekdayDate(shown.submitted_at), time: timeOf(shown.submitted_at) })}</span>
        <span className={shown.is_late ? s.late : "muted"}>{shown.is_late ? t("gb.late") : t("gb.onTime")}</span>
        <span className="muted">{t("grading.attempt", { n: shown.attempt })}</span>
      </p>
      {newestFirst.length > 1 && (
        <div role="tablist" aria-label={t("gw.attempts")} className={`tabs ${s.attempts}`}>
          {newestFirst.map((a) => (
            <button key={a.id} type="button" role="tab" aria-selected={a.id === shown.id} onClick={() => setPicked(a.attempt)}>{t("grading.attempt", { n: a.attempt })}</button>
          ))}
        </div>
      )}
      <Body item={item} attempt={shown} />
    </div>
  );
}

function Body({ item, attempt }: { item: Item; attempt: Attempt }) {
  const a = item.assignment;
  const type = a?.type;
  if (!a || (type !== "quiz" && type !== "form")) {
    return isDoc(attempt.content) ? <div className={`card ${s.work}`}><BlockRenderer doc={attempt.content} size={15} /></div> : null;
  }
  const config = a.config as unknown as QuizConfig & FormConfig;
  const raw = ((attempt.content as { answers?: Record<string, unknown> }).answers ?? {});
  const answers: AnswerMap = { ...answersFromWire(type, a.config, attempt.content) };
  const questions = type === "quiz" ? quizQuestions(config) : formQuestions(config, answers);
  // A long answer stays the document the student wrote, so that the teacher reads it formatted.
  for (const q of questions) if (q.kind === "long" && isDoc(raw[q.id])) answers[q.id] = raw[q.id] as Answer;

  // Staff see the score of each question the server scored; the key itself is not part of the attempt.
  const scores = (attempt.auto_scores ?? undefined) as Record<string, string> | undefined;
  const result: Record<string, QResult> | undefined = scores && Object.fromEntries(
    questions.flatMap((q) => {
      const got = scores[q.id];
      if (got === undefined || q.points === undefined) return [];
      const picked = answers[q.id];
      const chosen = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
      // The options of a fully scored question are all right; a partly scored one has no key to show, so none is marked.
      return [[q.id, { got, points: q.points, correct: toCents(got) === toCents(q.points) ? chosen : [], answer: null, feedback: null }] as const];
    }),
  );

  return (
    <QuestionSheet
      questions={questions}
      answers={answers}
      onChange={() => undefined}
      readOnly
      result={result}
      mode={type}
    />
  );
}
