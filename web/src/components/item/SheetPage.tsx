"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { plainText } from "@/components/doc/BlockRenderer";
import { Icon } from "@/components/Icon";
import { useCourseInfo, useCoursePeople } from "@/lib/api/hooks/courses";
import { useMySubmission, useQuizResult, useSaveDraft, useSubmitDraft } from "@/lib/api/hooks/assignments";
import { useMyGrades } from "@/lib/api/hooks/work";
import type { Answer, AnswerMap, FormConfig, QuizConfig } from "@/lib/api/gaps";
import { bodyOf, type Item } from "@/lib/api/types";
import { deadline } from "@/lib/assignment";
import { initials, timeOf } from "@/lib/format";
import { toCents } from "@/lib/points";
import { useMe } from "@/lib/session";
import { DueBar } from "./DueBar";
import { ItemFrame, PreviewBanner } from "./ItemFrame";
import { isAnswered, QuestionSheet, sheetStyles as qs, type Q, type QResult } from "./QuestionSheet";
import s from "./Item.module.css";

const short = (t: string) => (t.length > 26 ? `${t.slice(0, 24).trimEnd()}…` : t);

function quizQuestions(cfg: QuizConfig): Q[] {
  return cfg.questions.map((q) => ({ id: q.id, kind: q.type, label: q.prompt, short: short(q.prompt), points: q.points, options: q.options }));
}

function formQuestions(cfg: FormConfig, answers: AnswerMap): Q[] {
  return cfg.fields.map((f) => {
    const kind = f.type === "choice" ? (f.rules.multiple ? "multi" : "select") : f.type === "link" ? "url" : f.type;
    const mine = answers[f.id];
    const chosen = Array.isArray(mine) ? mine : typeof mine === "string" ? [mine] : [];
    // A variant another team took is not listed; the student's own pick stays (Form, "taken by other teams").
    const options = f.options.filter((o) => !o.taken_by || chosen.includes(o.id)).map((o) => ({ id: o.id, text: o.text }));
    const max = /(\d+)\s*$/.exec(String(f.rules.team_size ?? ""));
    return { id: f.id, kind: kind as Q["kind"], label: f.label, short: f.label, help: f.help || undefined, required: f.required, options, maxPeople: max ? Number(max[1]) : 3 };
  });
}

export function SheetPage({ item, courseId, staff, mode }: { item: Item; courseId: string; staff: boolean; mode: "quiz" | "form" }) {
  const a = item.assignment!;
  const me = useMe();
  const info = useCourseInfo(courseId);
  const people = useCoursePeople(courseId);
  const sub = useMySubmission(staff ? null : item.id);
  const grades = useMyGrades(staff || mode === "form" ? null : courseId);
  const grade = grades.data?.items.find((g) => g.assignment_id === item.id);
  const resultQ = useQuizResult(grade ? item.id : null);
  const save = useSaveDraft(item.id);
  const submit = useSubmitDraft(item.id);
  const [local, setLocal] = useState<AnswerMap | null>(null);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState<string[]>([]);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const dl = deadline(a);
  const attempts = sub.data?.attempts ?? [];
  const latest = attempts[0];
  const draft = sub.data?.draft ?? null;
  const attemptsLeft = a.max_attempts == null ? Infinity : a.max_attempts - attempts.length;
  const answersOf = (c: unknown): AnswerMap => ((c as { answers?: AnswerMap } | undefined)?.answers) ?? {};
  const view: "result" | "submitted" | "answering" | "closed" = grade && mode === "quiz" ? "result" : latest && !draft ? "submitted" : !latest && !draft && dl.closed ? "closed" : "answering";
  const editable = view === "answering" && !staff && dl.open;
  const loading = !staff && (!sub.data || (mode === "quiz" && !grades.data));

  const user = me.data;
  const config = a.config as unknown as QuizConfig & FormConfig;
  const base = local ?? (draft ? answersOf(draft.content) : latest ? answersOf(latest.content) : {});
  const answers: AnswerMap = { ...base };
  if (mode === "form" && user) for (const f of config.fields ?? []) if (f.type === "people" && answers[f.id] === undefined) answers[f.id] = [user.id];
  const questions = mode === "quiz" ? quizQuestions(config) : formQuestions(config, answers);

  async function flush(next: AnswerMap) {
    const r = await save.run({ answers: next });
    if (r) setSavedAt(r.updated_at);
  }
  function onChange(id: string, value: Answer) {
    const next = { ...answers, [id]: value };
    setLocal(next);
    setDismissed((d) => (d.includes(id) ? d : [...d, id]));
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void flush(next), 900);
  }
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);

  async function send() {
    if (timer.current) clearTimeout(timer.current);
    await flush(answers);
    const ok = await submit.run();
    setDismissed([]);
    if (ok) { setLocal(null); setSavedAt(null); }
  }
  async function edit() {
    await save.run({ answers: latest ? answersOf(latest.content) : {} });
    setLocal(null);
  }
  // Server errors show until the student edits that field.
  const fieldErrors = Object.fromEntries((submit.error?.errors ?? []).filter((e) => !dismissed.includes(e.field)).map((e) => [e.field, e.message]));

  const result = useMemo<Record<string, QResult> | undefined>(() => {
    if (!resultQ.data) return undefined;
    return Object.fromEntries(resultQ.data.questions.map((q) => [q.id, { got: q.got, points: q.points, correct: q.correct, answer: q.answer, feedback: q.feedback }]));
  }, [resultQ.data]);

  const answered = questions.filter((q) => isAnswered(q, answers[q.id])).length;
  const stamp = savedAt ?? draft?.updated_at;
  const noun = mode === "quiz" ? "quiz" : "form";
  const pill = {
    result: { label: "Graded", tone: "ok" as const },
    submitted: { label: "Submitted", tone: "ok" as const },
    closed: { label: "Not submitted", tone: "danger" as const },
    answering: mode === "quiz"
      ? { label: `${answered} of ${questions.length} answered${stamp ? `, saved ${timeOf(stamp)}` : ""}`, tone: "accent" as const }
      : stamp ? { label: `Draft saved ${timeOf(stamp)}`, tone: "accent" as const } : { label: "Not started", tone: "plain" as const },
  }[view];

  const owner = info.data?.owner_name ?? "your teacher";
  const [n = "", ...r] = owner.replace(/^Dr\. /, "").split(" ");
  const classmates = (people.data?.people ?? []).filter((p) => p.role === "student" && !p.former && p.user_id !== user?.id).map((p) => ({ id: p.user_id, name: `${p.name} ${p.surname}` }));
  const intro = mode === "form" ? config.note : plainText(bodyOf(item));
  const kindLabel = mode === "quiz" ? "Quiz" : "Form";
  const rows = questions.map((q, i) => {
    const res = result?.[q.id];
    const full = res && res.got === res.points;
    const has = isAnswered(q, answers[q.id]);
    return {
      q, i,
      state: res ? (full ? "full" : toCents(res.got) > 0 ? "partial" : "none") : has ? "answered" : "empty",
      mark: res ? res.got : view === "answering" && !has && !q.required && mode === "form" ? "" : view === "answering" && !has ? "Empty" : "",
      tone: res ? (full ? "ok" : "partial") : undefined,
    };
  });

  return (
    <ItemFrame courseId={courseId} itemId={item.id} focus>
      {({ crumbs }) => (
        <div className={qs.layout}>
          <nav aria-label={mode === "form" ? "Fields" : "Questions"} className={qs.nav}>
            <p className={qs.navTitle}>{view === "result" ? "Your points" : mode === "form" ? "Fields" : "Questions"}</p>
            <ol className={qs.navList}>
              {rows.map(({ q, i, state, mark, tone }) => (
                <li key={q.id}>
                  <a href={`#q${i + 1}`} className={qs.navLink}>
                    <span aria-hidden="true" className={qs.num} data-state={state}>{i + 1}</span>
                    <span className={qs.navShort}>{q.short}</span>
                    <span className={qs.navMark} data-tone={tone}>{mark}</span>
                  </a>
                </li>
              ))}
            </ol>
          </nav>
          <div className={qs.main}>
            {staff && <PreviewBanner courseId={courseId} itemId={item.id} />}
            {crumbs}
            <div className={s.kind}>
              <span aria-hidden="true" className={s.kindIcon} data-kind="assignment"><Icon name="assignment" size={15} strokeWidth={2} /></span>
              <span className={s.kindLabel} data-kind="assignment">Assignment</span>
              <span className="muted" style={{ fontSize: 14 }}>{kindLabel}</span>
            </div>
            <h1 className={s.title}>{item.title}</h1>
            <DueBar due={a.due_at ?? null} open={dl.open && view !== "result"} points={a.max_points} pill={pill} danger={view === "closed"} ungraded={mode === "form" && toCents(a.max_points) === 0}>
              {view === "result" && grade && <span className={s.score}><span className={s.scoreNum}>{grade.score}</span><span className="muted" style={{ fontSize: 14 }}> / {a.max_points}</span></span>}
              {editable && <button type="button" className="btn btn-primary" onClick={send} disabled={submit.pending}>Submit {noun}</button>}
              {view === "submitted" && mode === "form" && !staff && dl.open && attemptsLeft > 0 && <button type="button" className="btn btn-outline" onClick={edit} disabled={save.pending}>Edit and resubmit</button>}
            </DueBar>
            {submit.error && !submit.error.errors.length && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{submit.error.detail ?? submit.error.title}</p>}
            {view === "closed" && (
              <div role="status" className={s.note}>
                <strong>This {noun} is closed.</strong> You didn&apos;t submit before the deadline. If you think this is a mistake, write to {owner}.
              </div>
            )}
            {intro && <p style={{ margin: "20px 2px 20px", fontSize: 15 }}>
              {view === "result" ? (config.show_correct === false ? "Your teacher has published the grades." : "Your teacher has published the grades. Correct answers are shown because the quiz allows it.") : view === "submitted" ? `You submitted this ${noun}. ${mode === "quiz" ? "Your teacher will publish the grades." : ""}` : intro}
            </p>}
            {loading ? <p className="loading" role="status">Loading…</p> : (
              <>
                <QuestionSheet
                  questions={questions}
                  answers={answers}
                  onChange={onChange}
                  readOnly={!editable}
                  result={result}
                  errors={fieldErrors}
                  classmates={classmates}
                  me={user ? { id: user.id, name: `${user.name} ${user.surname}` } : undefined}
                  ownerInitials={initials(n, r.join(" "))}
                  ownerName={owner}
                  mode={mode}
                />
                {editable && (
                  <div className={`card ${qs.foot}`}>
                    <p className="muted" style={{ fontSize: 14, flex: "1 1 260px" }}>
                      {mode === "form" ? "You can edit and submit again until the deadline." : "Check your answers before you submit. After submitting you can’t change them."}
                    </p>
                    <button type="button" className="btn btn-primary btn-lg" onClick={send} disabled={submit.pending}>Submit {noun}</button>
                  </div>
                )}
              </>
            )}
          </div>
        </div>
      )}
    </ItemFrame>
  );
}
