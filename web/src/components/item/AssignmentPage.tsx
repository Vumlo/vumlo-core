"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BlockEditor } from "@/components/doc/BlockEditor";
import { BlockRenderer, plainText } from "@/components/doc/BlockRenderer";
import { emptyDoc } from "@/components/doc/model";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Icon } from "@/components/Icon";
import { errorText } from "@/lib/api/client";
import { useEditorProfiles } from "@/lib/api/hooks/content";
import { useCourseInfo } from "@/lib/api/hooks/courses";
import { useMySubmission, useSaveDraft, useSubmitDraft } from "@/lib/api/hooks/assignments";
import { useMyGrades } from "@/lib/api/hooks/work";
import { deadline } from "@/lib/assignment";
import { ownerName, typeLabel } from "@/lib/course";
import { dateTime, dayLong, initials, sinceLabel, timeOf } from "@/lib/format";
import { bodyOf, type Document, type Feature, type Item } from "@/lib/api/types";
import { attemptRule, lateRule } from "@/components/course/StudentContent";
import { DueBar } from "./DueBar";
import { ItemFrame, PreviewBanner } from "./ItemFrame";
import { useT } from "@/lib/i18n";
import { useQueued } from "@/lib/offline";
import { useTick } from "@/lib/useTick";
import { useUnloadGuard } from "@/lib/useUnloadGuard";
import s from "./Item.module.css";

const STUDENT_FEATURES: Feature[] = ["text_styles", "links", "lists", "code", "math"];

type View = "not-started" | "draft" | "focus" | "submitted" | "graded" | "closed";

export function AssignmentPage({ item, courseId, staff }: { item: Item; courseId: string; staff: boolean }) {
  const t = useT();
  const a = item.assignment!;
  const info = useCourseInfo(courseId);
  const sub = useMySubmission(staff ? null : item.id);
  const grades = useMyGrades(staff ? null : courseId);
  const profiles = useEditorProfiles();
  const save = useSaveDraft(item.id);
  const submit = useSubmitDraft(item.id);
  const waiting = useQueued(`/assignments/${item.id}/submission/submit`);
  const [focus, setFocus] = useState(false);
  const [doc, setDoc] = useState<Document | null>(null);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const unsaved = useRef<(() => void) | null>(null);
  const [asking, setAsking] = useState<string[] | null>(null);
  const features = profiles.data?.profiles.find((p) => p.name === "student")?.features ?? STUDENT_FEATURES;

  // The deadline can pass while the page is open.
  useTick();
  const dl = deadline(a);
  const attempts = sub.data?.attempts ?? [];
  const latest = attempts[0];
  const draft = sub.data?.draft ?? null;
  const grade = grades.data?.items.find((g) => g.assignment_id === item.id);
  const attemptsLeft = a.max_attempts == null ? Infinity : a.max_attempts - attempts.length;
  const canEdit = !staff && dl.open && attemptsLeft > 0 && !waiting;

  const view: View = grade ? "graded" : draft ? (focus ? "focus" : "draft") : latest ? "submitted" : dl.closed ? "closed" : "not-started";
  const loading = !staff && (!sub.data || !grades.data);

  // Autosave: the draft is stored a moment after the last change (ADR-0035, "saved as you type").
  const flush = useCallback(async (next: Document) => {
    const saved = await save.run(next);
    if (saved) setSavedAt(saved.updated_at);
    return !!saved;
  }, [save.run]);
  const onChange = useCallback((next: Document) => {
    setDoc(next);
    if (timer.current) clearTimeout(timer.current);
    unsaved.current = () => void flush(next);
    timer.current = setTimeout(() => { unsaved.current = null; void flush(next); }, 1200);
  }, [flush]);
  // Leaving the page does not drop what was typed in the last moment: it is sent as the page goes.
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); unsaved.current?.(); }, []);
  // A save that failed is tried again; the last version that was stored is kept meanwhile.
  useEffect(() => {
    if (!save.error || !doc) return;
    const retry = setTimeout(() => void flush(doc), 5000);
    return () => clearTimeout(retry);
  }, [save.error, doc, flush]);
  useUnloadGuard(() => unsaved.current !== null || save.pending || !!save.error);

  async function start() {
    await save.run(emptyDoc());
  }
  async function resubmit() {
    await save.run(latest ? (latest.content as unknown as Document) : emptyDoc());
  }
  // Sending cannot be undone, so an empty answer or the last attempt is confirmed first.
  function review() {
    const warnings: string[] = [];
    if (plainText(doc ?? (draft?.content as Document | undefined) ?? emptyDoc()).trim() === "") warnings.push(t("sub.empty"));
    if (attemptsLeft === 1) warnings.push(t("sub.last"));
    if (warnings.length > 0) setAsking(warnings);
    else void send();
  }
  async function send() {
    setAsking(null);
    if (timer.current) clearTimeout(timer.current);
    unsaved.current = null;
    // The server keeps the last stored draft: when the newest words did not reach it, nothing is sent.
    if (doc && !(await flush(doc))) return;
    const ok = await submit.run();
    if (ok) { setFocus(false); setDoc(null); setSavedAt(null); }
  }

  const initial = useMemo(() => (draft?.content as Document | undefined) ?? emptyDoc(), [draft?.content]);
  const editorKey = `${attempts.length}`;
  const stamp = savedAt ?? draft?.updated_at;
  const locked = !staff && !loading && (view === "draft" || view === "focus") && !dl.open;
  const problem = submit.error ?? (view === "draft" || view === "focus" ? undefined : save.error);
  const savedText = save.error ? t("asg.saveFailed") : save.pending ? t("asg.saving") : stamp ? t("asg.savedAt", { time: timeOf(stamp) }) : t("asg.notSaved");

  const pills = {
    "not-started": { label: t("status.notStarted"), tone: "plain" as const },
    draft: { label: stamp ? t("asg.pillSaved", { time: timeOf(stamp) }) : t("common.draft"), tone: "accent" as const },
    focus: { label: stamp ? t("asg.pillSaved", { time: timeOf(stamp) }) : t("common.draft"), tone: "accent" as const },
    submitted: { label: a.max_attempts ? t("asg.submittedOf", { n: latest?.attempt ?? 1, max: a.max_attempts }) : t("asg.submittedN", { n: latest?.attempt ?? 1 }), tone: "ok" as const },
    graded: { label: t("status.graded"), tone: "ok" as const },
    closed: { label: t("status.notSubmitted"), tone: "danger" as const },
  };
  // Until the submission is known, "Not started" or "Not submitted" would be a guess.
  const pill = loading ? { label: t("common.loading"), tone: "plain" as const } : pills[view];

  const instructions = (
    <section aria-labelledby="instr-h">
      <h2 id="instr-h" className={s.h2}>{t("item.instructions")}</h2>
      <BlockRenderer doc={bodyOf(item)} size={16} />
      <ul className={s.facts}>
        {a.max_attempts != null && <li>{t.plural("asg.canSubmit", a.max_attempts)} {t("asg.gradesLast")}</li>}
        {a.max_attempts == null && <li>{attemptRule(null)} {t("asg.gradesLast")}</li>}
        <li>{lateRule(a)}</li>
        {a.on_deadline === "auto_submit" && <li>{t("asg.autoSubmit")}</li>}
      </ul>
    </section>
  );

  return (
    <ItemFrame courseId={courseId} itemId={item.id} focus={view === "focus"}>
      {({ crumbs }) => (
        <>
          {staff && <PreviewBanner courseId={courseId} itemId={item.id} />}
          {crumbs}
          <div className={s.kind}>
            <span aria-hidden="true" className={s.kindIcon} data-kind="assignment"><Icon name="assignment" size={15} strokeWidth={2} /></span>
            <span className={s.kindLabel} data-kind="assignment">{t("cm.assignment")}</span>
            <span className="muted" style={{ fontSize: 14 }}>{typeLabel(a.type)}</span>
          </div>
          <h1 className={s.title}>{item.title}</h1>

          <DueBar due={a.due_at ?? null} open={dl.open && view !== "graded"} points={a.max_points} pill={pill} danger={!loading && (view === "closed" || locked)}>
            {!loading && view === "not-started" && canEdit && <button type="button" className="btn btn-primary" onClick={start} disabled={save.pending}>{t("asg.start")}</button>}
            {(view === "draft" || view === "focus") && !locked && <button type="button" className="btn btn-submit" onClick={review} disabled={submit.pending || save.pending || waiting}><Icon name="check" size={16} strokeWidth={2.4} />{t("asg.submit")}</button>}
            {view === "graded" && grade && (
              <span className={s.score}><span className={s.scoreNum}>{grade.score}</span><span className="muted" style={{ fontSize: 14 }}> / {a.max_points}</span></span>
            )}
          </DueBar>
          {waiting && <p role="status" className="alert alert-info" style={{ marginTop: 12 }}>{t("offline.queuedNote")}</p>}
          {problem && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{errorText(problem)}</p>}
          {asking && (
            <ConfirmDialog title={t("sub.title")} confirm={t("asg.submit")} onClose={() => setAsking(null)} onConfirm={() => void send()}>
              {asking.join(" ")}
            </ConfirmDialog>
          )}

          {!loading && (view === "closed" || locked) && (
            <div role="status" className={s.note}>
              <strong>{t("asg.closedTitle")}</strong> {t("asg.closedLead", { owner: ownerName(info.data) ?? t("item.yourTeacher") })}
            </div>
          )}

          {loading ? <p className="loading" role="status">{t("common.loading")}</p> : (
            <div className={s.blocks}>
              {view !== "focus" && instructions}

              {(view === "draft" || view === "focus") && (
                <div className={view === "focus" ? s.answerFocus : s.answer}>
                  {view === "focus" && (
                    <button type="button" className="btn" style={{ marginBottom: 12 }} onClick={() => setFocus(false)}>
                      <Icon name="text" size={16} strokeWidth={2} />
                      {t("asg.showInstructions")}
                    </button>
                  )}
                  <section aria-labelledby="answer-h">
                    <div className="row" style={{ alignItems: "baseline", justifyContent: "space-between", gap: 12, marginBottom: 12 }}>
                      <h2 id="answer-h" className={s.h2} style={{ margin: 0 }}>{t("asg.yourAnswer")}</h2>
                      <span className="row" style={{ gap: 12 }}>
                        <span role="status" className={s.saved} style={save.error ? { color: "var(--danger)" } : undefined}><Icon name={save.error ? "info" : "check"} size={14} strokeWidth={2.2} />{savedText}</span>
                        <button type="button" className="btn btn-quiet" style={{ minHeight: 36, color: "var(--muted)", fontSize: 13, padding: "0 12px" }} aria-label={view === "focus" ? t("asg.exitFocusLabel") : t("asg.focusLabel")} onClick={() => setFocus(view !== "focus")}>
                          <Icon name={view === "focus" ? "focusExit" : "focus"} size={16} strokeWidth={2} />
                          {view === "focus" ? t("asg.exitFocus") : t("asg.focus")}
                        </button>
                      </span>
                    </div>
                    {locked ? <BlockRenderer doc={initial} size={16} /> : (
                      <div className={s.editor}>
                        <BlockEditor
                          key={editorKey}
                          value={initial}
                          onChange={onChange}
                          features={features}
                          label={t("asg.yourAnswer")}
                          mode="float"
                          minHeight={view === "focus" ? 560 : 320}
                          placeholder={t("asg.placeholder")}
                        />
                      </div>
                    )}
                    <p className="muted" style={{ margin: "12px 4px 0", fontSize: 14 }}>
                      {t("asg.savedAsYouType")} {a.max_attempts ? t("asg.usesOf", { n: attempts.length + 1, max: a.max_attempts }) : t("asg.uses", { n: attempts.length + 1 })}
                    </p>
                  </section>
                </div>
              )}

              {view === "submitted" && latest && (
                <section aria-labelledby="sub-h" className={s.panel}>
                  <div className={s.panelHead}>
                    <div>
                      <h2 id="sub-h" style={{ fontSize: 16, fontWeight: 600 }}>{t("grading.attempt", { n: latest.attempt })}</h2>
                      <p className="muted" style={{ fontSize: 13 }}>{t("asg.submittedWaiting", { when: sinceLabel(latest.submitted_at) })}</p>
                    </div>
                    {canEdit && <button type="button" className="btn btn-outline" onClick={resubmit} disabled={save.pending}>{t("asg.resubmit")}</button>}
                  </div>
                  <div className={s.panelBody}><BlockRenderer doc={latest.content as unknown as Document} size={16} /></div>
                  {canEdit && (
                    <p className={s.panelFoot}>
                      {attemptsLeft === Infinity ? t("asg.again") : t.plural("asg.left", attemptsLeft)} {t("asg.resubmitNote")}
                    </p>
                  )}
                </section>
              )}

              {view === "graded" && grade && (
                <section aria-labelledby="grade-h" className={s.panel}>
                  <div className={s.gradeTop}>
                    <div className="row" style={{ flexWrap: "wrap", alignItems: "baseline", justifyContent: "space-between", gap: 8 }}>
                      <h2 id="grade-h" style={{ fontSize: 16, fontWeight: 600 }}>{t("item.grade")}</h2>
                      <span className="muted" style={{ fontSize: 13 }}>{t("item.publishedOn", { day: dayLong(grade.published_at) })}</span>
                    </div>
                    <p style={{ marginTop: 8, whiteSpace: "nowrap" }}><span className={s.gradeNum}>{grade.score}</span><span className="muted" style={{ fontSize: 17 }}> / {a.max_points}</span></p>
                  </div>
                  {grade.feedback && (
                    <div className={s.comment}>
                      <div className="row" style={{ gap: 12, marginBottom: 12 }}>
                        <span aria-hidden="true" className={s.avatar}>{initials(info.data?.owner_name ?? "", info.data?.owner_surname ?? "")}</span>
                        <div>
                          <p style={{ fontSize: 14, fontWeight: 600 }}>{ownerName(info.data)}</p>
                          <p className="muted" style={{ fontSize: 12 }}>{latest ? t("asg.commentOn", { n: latest.attempt }) : t("asg.comment")}</p>
                        </div>
                      </div>
                      <p style={{ fontSize: 16, lineHeight: 1.65 }}>{grade.feedback}</p>
                    </div>
                  )}
                  {latest && (
                    <details className={s.details}>
                      <summary>
                        <Icon name="chevronRight" size={16} strokeWidth={2} style={{ color: "var(--muted)" }} />
                        {t("asg.yourAnswer")}
                        <span className="muted" style={{ fontWeight: 400 }}>{t("asg.attemptSubmitted", { n: latest.attempt, when: dateTime(latest.submitted_at) })}</span>
                      </summary>
                      <div><BlockRenderer doc={latest.content as unknown as Document} size={15} /></div>
                    </details>
                  )}
                </section>
              )}
            </div>
          )}
        </>
      )}
    </ItemFrame>
  );
}
