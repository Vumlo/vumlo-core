"use client";

import { useRouter } from "next/navigation";
import { useEffect, useMemo, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { plainText } from "@/components/doc/BlockRenderer";
import { Icon } from "@/components/Icon";
import { errorText } from "@/lib/api/client";
import { useCourseInfo, useCoursePeople } from "@/lib/api/hooks/courses";
import { useMySubmission, useQuizResult, useSaveDraft, useSubmitDraft } from "@/lib/api/hooks/assignments";
import { useMyGrades } from "@/lib/api/hooks/work";
import type { Answer, AnswerMap, FormConfig, QuizConfig } from "@/lib/api/gaps";
import { answersFromWire, answersToWire } from "@/lib/api/sheetWire";
import { bodyOf, type Item } from "@/lib/api/types";
import { deadline } from "@/lib/assignment";
import { ownerName } from "@/lib/course";
import { initials, timeOf } from "@/lib/format";
import { toCents } from "@/lib/points";
import { useMe } from "@/lib/session";
import { DueBar } from "./DueBar";
import { ItemFrame, PreviewBanner } from "./ItemFrame";
import { isAnswered, QuestionSheet, sheetStyles as qs, type Q, type QResult } from "./QuestionSheet";
import { useT } from "@/lib/i18n";
import { useOffline, useQueued } from "@/lib/offline";
import { useTick } from "@/lib/useTick";
import { useUnloadGuard } from "@/lib/useUnloadGuard";
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
  const t = useT();
  const a = item.assignment!;
  const me = useMe();
  const info = useCourseInfo(courseId);
  const people = useCoursePeople(courseId);
  const sub = useMySubmission(staff ? null : item.id);
  const grades = useMyGrades(staff || mode === "form" ? null : courseId);
  const grade = grades.data?.items.find((g) => g.assignment_id === item.id);
  const config = a.config as unknown as QuizConfig & FormConfig;
  const resultQ = useQuizResult(grade ? item.id : null, config);
  // A form waits for a connection in the send queue; a quiz does not: it is timed and counted by the server (ADR-0060).
  const save = useSaveDraft(item.id, mode === "form");
  const submit = useSubmitDraft(item.id, mode === "form");
  const waiting = useQueued(`/assignments/${item.id}/submission/submit`);
  const offline = useOffline();
  const [local, setLocal] = useState<AnswerMap | null>(null);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState<string[]>([]);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const unsaved = useRef<(() => void) | null>(null);

  // The deadline can pass while the page is open.
  useTick();
  const [asking, setAsking] = useState<string[] | null>(null);
  const dl = deadline(a);
  const attempts = sub.data?.attempts ?? [];
  const latest = attempts[0];
  const draft = sub.data?.draft ?? null;
  const attemptsLeft = a.max_attempts == null ? Infinity : a.max_attempts - attempts.length;
  const answersOf = (c: unknown): AnswerMap => answersFromWire(mode, a.config, c);
  const view: "result" | "submitted" | "answering" | "closed" = grade && mode === "quiz" ? "result" : latest && !draft ? "submitted" : !latest && !draft && dl.closed ? "closed" : "answering";
  const editable = view === "answering" && !staff && dl.open && !waiting && !(mode === "quiz" && offline);
  const loading = !staff && (!sub.data || (mode === "quiz" && !grades.data));

  const user = me.data;
  const base = local ?? (draft ? answersOf(draft.content) : latest ? answersOf(latest.content) : {});
  const answers: AnswerMap = { ...base };
  if (mode === "form" && user) for (const f of config.fields ?? []) if (f.type === "people" && answers[f.id] === undefined) answers[f.id] = [user.id];
  const questions = mode === "quiz" ? quizQuestions(config) : formQuestions(config, answers);

  async function flush(next: AnswerMap) {
    const r = await save.run({ answers: answersToWire(mode, a.config, next) });
    if (r) setSavedAt(r.updated_at);
    return !!r;
  }
  function onChange(id: string, value: Answer) {
    const next = { ...answers, [id]: value };
    setLocal(next);
    setDismissed((d) => (d.includes(id) ? d : [...d, id]));
    if (timer.current) clearTimeout(timer.current);
    unsaved.current = () => void flush(next);
    timer.current = setTimeout(() => { unsaved.current = null; void flush(next); }, 900);
  }
  // Leaving the page does not drop what was typed in the last moment: it is sent as the page goes.
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); unsaved.current?.(); }, []);

  // A failed save is tried again; the last stored copy is kept meanwhile.
  useEffect(() => {
    if (!save.error || !local) return;
    const retry = setTimeout(() => void flush(local), 5000);
    return () => clearTimeout(retry);
  });
  useUnloadGuard(() => unsaved.current !== null || save.pending || !!save.error);

  // Sending cannot be undone, so unanswered questions and the last attempt are confirmed first.
  function review() {
    const warnings: string[] = [];
    const open = mode === "quiz" ? questions.length - questions.filter((q) => isAnswered(q, answers[q.id])).length : 0;
    if (open > 0) warnings.push(t.plural("sub.unanswered", open));
    if (mode === "quiz" && attemptsLeft === 1) warnings.push(t("sub.last"));
    if (warnings.length > 0) setAsking(warnings);
    else void send();
  }
  async function send() {
    setAsking(null);
    if (timer.current) clearTimeout(timer.current);
    unsaved.current = null;
    // The server keeps the last stored draft: when the newest answers did not reach it, nothing is sent.
    if (!(await flush(answers))) return;
    const ok = await submit.run();
    setDismissed([]);
    if (ok) { setLocal(null); setSavedAt(null); }
  }
  async function edit() {
    await save.run({ answers: answersToWire(mode, a.config, latest ? answersOf(latest.content) : {}) });
    setLocal(null);
  }
  // Server errors show until the student edits that field.
  const fieldErrors = Object.fromEntries((submit.error?.errors ?? []).map((e) => [e.field.replace(/^content\.answers\./, ""), e.message] as const).filter(([id]) => !dismissed.includes(id)));

  const result = useMemo<Record<string, QResult> | undefined>(() => {
    if (!resultQ.data) return undefined;
    return Object.fromEntries(resultQ.data.questions.map((q) => [q.id, { got: q.got, points: q.points, correct: q.correct, answer: q.answer, feedback: q.feedback }]));
  }, [resultQ.data]);

  const answered = questions.filter((q) => isAnswered(q, answers[q.id])).length;
  const stamp = savedAt ?? draft?.updated_at;

  // A student who has started and not submitted is asked before a click takes them to another page. Nothing is lost
  // (answers are saved as they go), but the work only counts once it is submitted.
  const router = useRouter();
  const [leaving, setLeaving] = useState<string | null>(null);
  const warnLeave = editable && answered > 0;
  useEffect(() => {
    if (!warnLeave) return;
    const onClick = (e: MouseEvent) => {
      const link = (e.target instanceof Element ? e.target.closest("a[href]") : null) as HTMLAnchorElement | null;
      if (!link || e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || link.target === "_blank") return;
      const url = new URL(link.href, window.location.href);
      if (url.origin !== window.location.origin || (url.pathname === window.location.pathname && url.search === window.location.search)) return;
      e.preventDefault();
      e.stopPropagation();
      setLeaving(url.pathname + url.search + url.hash);
    };
    document.addEventListener("click", onClick, true);
    return () => document.removeEventListener("click", onClick, true);
  }, [warnLeave]);
  const pills = {
    result: { label: t("status.graded"), tone: "ok" as const },
    submitted: { label: t("status.submitted"), tone: "ok" as const },
    closed: { label: t("status.notSubmitted"), tone: "danger" as const },
    answering: mode === "quiz"
      ? { label: `${t("sheet.answered", { n: answered, total: questions.length })}${stamp ? `, ${t("sheet.saved", { time: timeOf(stamp) })}` : ""}`, tone: "accent" as const }
      : stamp ? { label: t("asg.pillSaved", { time: timeOf(stamp) }), tone: "accent" as const } : { label: t("status.notStarted"), tone: "plain" as const },
  };
  // Until the submission is known, "Not started" or "Not submitted" would be a guess.
  const pill = loading ? { label: t("common.loading"), tone: "plain" as const } : pills[view];
  // A draft that was never sent after the deadline can no longer be changed or sent.
  const locked = !staff && !loading && view === "answering" && !dl.open;

  const owner = ownerName(info.data) ?? t("item.yourTeacher");
  const classmates = (people.data?.people ?? []).filter((p) => p.role === "student" && !p.former && p.user_id !== user?.id).map((p) => ({ id: p.user_id, name: `${p.name} ${p.surname}` }));
  const intro = mode === "form" ? config.note : plainText(bodyOf(item));
  const kindLabel = mode === "quiz" ? t("type.quiz") : t("type.form");
  const rows = questions.map((q, i) => {
    const res = result?.[q.id];
    const full = res && res.got === res.points;
    const has = isAnswered(q, answers[q.id]);
    return {
      q, i,
      state: res ? (full ? "full" : toCents(res.got) > 0 ? "partial" : "none") : has ? "answered" : "empty",
      mark: res ? res.got : view === "answering" && !has && !q.required && mode === "form" ? "" : view === "answering" && !has ? t("sheet.empty") : "",
      tone: res ? (full ? "ok" : "partial") : undefined,
    };
  });

  return (
    <ItemFrame courseId={courseId} itemId={item.id} focus keepOutline>
      {({ crumbs }) => (
        <div className={qs.layout}>
          <div className={qs.main}>
            {staff && <PreviewBanner courseId={courseId} itemId={item.id} />}
            {crumbs}
            <div className={s.kind}>
              <span aria-hidden="true" className={s.kindIcon} data-kind="assignment"><Icon name="assignment" size={15} strokeWidth={2} /></span>
              <span className={s.kindLabel} data-kind="assignment">{t("cm.assignment")}</span>
              <span className="muted" style={{ fontSize: 14 }}>{kindLabel}</span>
            </div>
            <h1 className={s.title}>{item.title}</h1>
            {waiting && <p role="status" className="alert alert-info" style={{ marginTop: 12 }}>{t("offline.queuedNote")}</p>}
            {mode === "quiz" && offline && view === "answering" && !staff && <p role="status" className="alert alert-info" style={{ marginTop: 12 }}>{t("offline.quizNeeds")}</p>}
            {submit.error && !submit.error.errors.length && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{errorText(submit.error)}</p>}
            {save.error && view === "answering" && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{t("sheet.saveFailed")}</p>}
            {asking && (
              <ConfirmDialog title={t("sub.title")} confirm={t(mode === "quiz" ? "sheet.submitQuiz" : "sheet.submitForm")} onClose={() => setAsking(null)} onConfirm={() => void send()}>
                {asking.join(" ")}
              </ConfirmDialog>
            )}
            {!loading && (view === "closed" || locked) && (
              <div role="status" className={s.note}>
                <strong>{t(mode === "quiz" ? "sheet.closedQuiz" : "sheet.closedForm")}</strong> {t("sheet.closedLead", { owner })}
              </div>
            )}
            {intro && <p style={{ margin: "20px 4px 20px", fontSize: 15 }}>
              {view === "result" ? (config.show_correct === false ? t("sheet.published") : t("sheet.publishedShown")) : view === "submitted" ? `${t(mode === "quiz" ? "sheet.youSubmittedQuiz" : "sheet.youSubmittedForm")} ${mode === "quiz" ? t("sheet.willPublish") : ""}` : intro}
            </p>}
            {loading ? <p className="loading" role="status">{t("common.loading")}</p> : (
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
                  ownerInitials={initials(info.data?.owner_name ?? "", info.data?.owner_surname ?? "")}
                  ownerName={owner}
                  mode={mode}
                />
                {editable && (
                  <div className={`card ${qs.foot}`}>
                    <p className="muted" style={{ fontSize: 14, flex: "1 1 260px" }}>
                      {mode === "form" ? t("sheet.footForm") : t("sheet.footQuiz")}
                    </p>
                    <button type="button" className="btn btn-submit btn-lg" onClick={review} disabled={submit.pending}><Icon name="check" size={16} strokeWidth={2.4} />{t(mode === "quiz" ? "sheet.submitQuiz" : "sheet.submitForm")}</button>
                  </div>
                )}
              </>
            )}
          </div>
          <aside aria-label={t("sheet.overview")} className={qs.panel}>
            <DueBar stacked due={a.due_at ?? null} open={dl.open && view !== "result"} points={a.max_points} pill={pill} danger={!loading && (view === "closed" || locked)} ungraded={mode === "form" && toCents(a.max_points) === 0}>
              {view === "result" && grade && <span className={s.score}><span className={s.scoreNum}>{grade.score}</span><span className="muted" style={{ fontSize: 14 }}> / {a.max_points}</span></span>}
              {editable && <button type="button" className="btn btn-submit" onClick={review} disabled={submit.pending}><Icon name="check" size={16} strokeWidth={2.4} />{t(mode === "quiz" ? "sheet.submitQuiz" : "sheet.submitForm")}</button>}
              {view === "submitted" && mode === "form" && !staff && dl.open && attemptsLeft > 0 && <button type="button" className="btn btn-outline" onClick={edit} disabled={save.pending}>{t("asg.resubmit")}</button>}
            </DueBar>
            <nav aria-label={mode === "form" ? t("sheet.fields") : t("sheet.questions")} className={qs.nav}>
              <p className={qs.navTitle}>{view === "result" ? t("sheet.yourPoints") : mode === "form" ? t("sheet.fields") : t("sheet.questions")}</p>
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
          </aside>
          {leaving && (
            <ConfirmDialog title={t("sheet.leaveTitle")} confirm={t("sheet.leaveConfirm")} onClose={() => setLeaving(null)} onConfirm={() => { const to = leaving; setLeaving(null); router.push(to); }}>
              {t("sheet.leaveBody")}
            </ConfirmDialog>
          )}
        </div>
      )}
    </ItemFrame>
  );
}
