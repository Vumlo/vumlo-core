"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { Icon } from "@/components/Icon";
import { newId } from "@/components/doc/model";
import { usePublishItem, useUpdateItem } from "@/lib/api/hooks/content";
import { useAutosave } from "@/lib/autosave";
import type { QuizConfig, QuizQuestion, QuizQuestionType } from "@/lib/api/gaps";
import type { Item } from "@/lib/api/types";
import { dueLabel } from "@/lib/format";
import { normalizePoints, sumPoints, toCents } from "@/lib/points";
import { AssignmentSettings } from "./AssignmentSettings";
import { AuthorBar, Switch } from "./AuthorBar";
import { translate } from "@/lib/i18n/core";
import type { MessageKey } from "@/lib/i18n/core";
import { useT } from "@/lib/i18n";
import s from "./Author.module.css";
import { itemHref } from "@/lib/routes";

const TYPES: [QuizQuestionType, MessageKey][] = [["single", "qb.single"], ["multi", "qb.multi"], ["number", "qb.number"], ["short", "qb.short"], ["long", "qb.long"]];
const label = (type: QuizQuestionType) => translate(TYPES.find(([k]) => k === type)![1]);
const short = (t: string) => (t.length > 28 ? `${t.slice(0, 26).trimEnd()}…` : t || translate("qb.newQuestion"));

function blank(type: QuizQuestionType): QuizQuestion {
  const opts = type === "single" || type === "multi" ? [{ id: newId(), text: "" }, { id: newId(), text: "" }] : [];
  return { id: newId(), type, prompt: "", points: "1.00", options: opts, correct: [], ...(type === "number" ? { answer: "", tolerance: "0" } : {}) };
}

function summary(q: QuizQuestion): string {
  if (q.type === "single" || q.type === "multi") {
    const names = q.options.filter((o) => q.correct?.includes(o.id)).map((o) => o.text || translate("qb.empty"));
    return `${translate("qb.correctList", { list: names.join(", ") || translate("qb.noneChosen") })}${q.type === "multi" && q.partial_credit ? ` ${translate("qb.partialCredit")}.` : ""}`;
  }
  if (q.type === "number") return translate("qb.correctList", { list: q.answer || translate("qb.notSet") });
  return translate("qb.selfGrade");
}

export function QuizBuilder({ item, courseId }: { item: Item; courseId: string }) {
  const t = useT();
  const update = useUpdateItem(courseId);
  const publish = usePublishItem(courseId);
  const [title, setTitle] = useState(item.title);
  const [cfg, setCfg] = useState<QuizConfig>(() => (item.assignment!.config as unknown as QuizConfig).questions ? (item.assignment!.config as unknown as QuizConfig) : { questions: [], shuffle_questions: false, show_correct: true });
  const [sel, setSel] = useState(0);
  const [tab, setTab] = useState<"question" | "quiz">("question");
  const [showSettings, setShowSettings] = useState(false);

  const total = useMemo(() => sumPoints(cfg.questions.map((q) => (Number.isNaN(toCents(q.points)) ? "0" : q.points))), [cfg.questions]);
  const draft = useMemo(() => ({ title, cfg, total }), [title, cfg, total]);
  const auto = useAutosave(draft, (d) => update.run(item.id, { title: d.title.trim() || item.title, config: d.cfg as unknown as Record<string, unknown>, configType: "quiz", ...(toCents(d.total) > 0 ? { max_points: d.total } : {}) }));
  const a = item.assignment!;
  const isDraft = item.status === "draft";
  const q = cfg.questions[Math.min(sel, cfg.questions.length - 1)];
  const qi = q ? cfg.questions.indexOf(q) : -1;

  const setQ = (i: number, fn: (x: QuizQuestion) => QuizQuestion) => setCfg((c) => ({ ...c, questions: c.questions.map((x, k) => (k === i ? fn(x) : x)) }));
  const add = (t: QuizQuestionType) => { setCfg((c) => ({ ...c, questions: [...c.questions, blank(t)] })); setSel(cfg.questions.length); setTab("question"); };
  const changeType = (t: QuizQuestionType) => setQ(qi, (x) => {
    const choice = t === "single" || t === "multi";
    const options = choice ? (x.options.length >= 2 ? x.options : [{ id: newId(), text: "" }, { id: newId(), text: "" }]) : [];
    return { ...x, type: t, options, correct: t === "single" ? (x.correct ?? []).slice(0, 1) : choice ? x.correct : [], ...(t === "number" ? { answer: x.answer ?? "", tolerance: x.tolerance ?? "0" } : {}) };
  });
  const toggleCorrect = (qq: QuizQuestion, id: string) => (qq.type === "single" ? [id] : (qq.correct ?? []).includes(id) ? (qq.correct ?? []).filter((x) => x !== id) : [...(qq.correct ?? []), id]);
  const pointsError = q && q.points.trim() !== "" && !/^\d+(\.\d{1,2})?$/.test(q.points.trim());

  return (
    <div className="container" style={{ paddingTop: 20 }}>
      <AuthorBar courseId={courseId} itemId={item.id} title={title} chip={isDraft ? { text: t("le.draftChip"), tone: "draft" } : { text: t("cm.published"), tone: "ok" }} saved={auto.savedAt} pending={auto.pending} failed={auto.failed}>
        <Link href={itemHref(courseId, item.id)} className="btn">{t("cm.preview")}</Link>
        {isDraft && <button type="button" className="btn btn-primary" disabled={publish.pending} onClick={async () => { await auto.flush(); await publish.run(item.id); }}>{t("qb.publish")}</button>}
      </AuthorBar>
      <div className={s.shell}>
        <nav aria-label={t("sheet.questions")} className={`card ${s.qnav}`}>
          <div className="row" style={{ justifyContent: "space-between" }}><h2 style={{ fontSize: 15, fontWeight: 600 }}>{t("sheet.questions")}</h2><span className="muted" style={{ fontSize: 13 }}>{t("qb.pts", { n: total })}</span></div>
          <ol>
            {cfg.questions.map((x, i) => (
              <li key={x.id}>
                <button type="button" className={s.qnavBtn} aria-current={i === qi ? "step" : undefined} onClick={() => { setSel(i); setTab("question"); }}>
                  <span className={s.qnum}>{i + 1}</span><span className={s.qshort}>{short(x.prompt)}</span><span className="muted" style={{ fontSize: 12 }}>{x.points}</span>
                </button>
              </li>
            ))}
          </ol>
          <button type="button" className="btn btn-block" onClick={() => add("single")}><Icon name="plus" size={15} strokeWidth={2.2} />{t("qb.addQuestion")}</button>
          <p className="muted" style={{ marginTop: 10, fontSize: 13 }}>{t("qb.addsUp", { n: total })}</p>
        </nav>

        <div className={s.canvas}>
          <p className="muted" style={{ fontSize: 14, marginBottom: 8 }}><strong style={{ color: "var(--asg-fg)" }}>{t("cm.assignment")}</strong> {t("type.quiz")}{a.due_at ? `, ${t("search.due", { when: dueLabel(a.due_at) })}` : ""}, {t("row.points", { points: total })}</p>
          <label htmlFor="qtitle" className="sr-only">{t("qb.quizTitle")}</label>
          <input id="qtitle" className={s.titleInput} value={title} onChange={(e) => setTitle(e.target.value)} />
          <div className="stack" style={{ gap: 14, marginTop: 16 }}>
            {cfg.questions.map((x, i) => {
              const selected = i === qi;
              const choice = x.type === "single" || x.type === "multi";
              return (
                <article key={x.id} aria-label={t("sheet.question", { n: i + 1 })} className={s.card} data-selected={selected} onClick={() => { if (!selected) setSel(i); }}>
                  <div className={s.cardHead}><span className={s.qnum}>{i + 1}</span><strong>{label(x.type)}</strong><span style={{ marginLeft: "auto" }}>{t("qb.pts", { n: x.points })}</span></div>
                  {!selected ? (
                    <>
                      <p style={{ fontSize: 15, fontWeight: 500 }}>{x.prompt || <span className="muted">{t("qb.newQuestion")}</span>}</p>
                      <p className="muted" style={{ fontSize: 13 }}>{summary(x)}</p>
                    </>
                  ) : (
                    <div className="stack" style={{ gap: 12 }}>
                      <div className="field">
                        <label htmlFor={`prompt-${x.id}`}>{t("qb.question")}</label>
                        <textarea id={`prompt-${x.id}`} className="textarea" style={{ minHeight: 70 }} value={x.prompt} onChange={(e) => setQ(i, (y) => ({ ...y, prompt: e.target.value }))} />
                      </div>
                      {choice && (
                        <div>
                          <p style={{ fontSize: 14, fontWeight: 500, marginBottom: 8 }}>{x.type === "multi" ? t("qb.optionsMulti") : t("qb.optionsSingle")}</p>
                          <div role="group" aria-label={t("qb.options")} className="stack" style={{ gap: 8 }}>
                            {x.options.map((o, oi) => {
                              const correct = (x.correct ?? []).includes(o.id);
                              return (
                                <div key={o.id} className={s.optRow} data-correct={correct}>
                                  <button type="button" role={x.type === "multi" ? "checkbox" : "radio"} aria-checked={correct} aria-label={t("qb.markCorrect", { n: oi + 1 })} className={s.mark} data-shape={x.type === "multi" ? "check" : "radio"} onClick={() => setQ(i, (y) => ({ ...y, correct: toggleCorrect(y, o.id) }))}>
                                    {correct && <Icon name="check" size={13} strokeWidth={3} />}
                                  </button>
                                  <input type="text" aria-label={t("qb.option", { n: oi + 1 })} value={o.text} placeholder={t("qb.option", { n: oi + 1 })} onChange={(e) => setQ(i, (y) => ({ ...y, options: y.options.map((z) => (z.id === o.id ? { ...z, text: e.target.value } : z)) }))} />
                                  {correct && <span style={{ fontSize: 12, fontWeight: 600, color: "var(--ok-fg)" }}>{t("qb.correct")}</span>}
                                  <button type="button" className="icon-btn" style={{ width: 32, height: 32 }} aria-label={t("qb.removeOption", { n: oi + 1 })} disabled={x.options.length <= 2} onClick={() => setQ(i, (y) => ({ ...y, options: y.options.filter((z) => z.id !== o.id), correct: (y.correct ?? []).filter((c) => c !== o.id) }))}><Icon name="close" size={15} strokeWidth={2} /></button>
                                </div>
                              );
                            })}
                          </div>
                          <button type="button" className="btn" style={{ marginTop: 8, minHeight: 36 }} disabled={x.options.length >= 10} onClick={() => setQ(i, (y) => ({ ...y, options: [...y.options, { id: newId(), text: "" }] }))}><Icon name="plus" size={15} strokeWidth={2.2} />{t("qb.addOption")}</button>
                        </div>
                      )}
                      {x.type === "number" && (
                        <div className="row" style={{ gap: 14, flexWrap: "wrap" }}>
                          <div className="field"><label htmlFor={`ans-${x.id}`}>{t("sheet.correct")}</label><input id={`ans-${x.id}`} className="input" style={{ maxWidth: 160 }} inputMode="decimal" value={x.answer ?? ""} onChange={(e) => setQ(i, (y) => ({ ...y, answer: e.target.value }))} /></div>
                          <div className="field"><label htmlFor={`tol-${x.id}`}>{t("qb.within")}</label><input id={`tol-${x.id}`} className="input" style={{ maxWidth: 120 }} inputMode="decimal" value={x.tolerance ?? "0"} onChange={(e) => setQ(i, (y) => ({ ...y, tolerance: e.target.value }))} /></div>
                        </div>
                      )}
                      {(x.type === "long" || x.type === "short") && (
                        <div className="field">
                          <label htmlFor={`notes-${x.id}`}>{t("qb.notes")}</label>
                          <textarea id={`notes-${x.id}`} className="textarea" style={{ minHeight: 70 }} value={x.grading_notes ?? ""} onChange={(e) => setQ(i, (y) => ({ ...y, grading_notes: e.target.value }))} />
                          <p className="hint">{t("qb.notesHint")}</p>
                        </div>
                      )}
                    </div>
                  )}
                </article>
              );
            })}
            {cfg.questions.length === 0 && <p className="empty">{t("qb.none")}</p>}
          </div>
          <div className="row" style={{ gap: 8, flexWrap: "wrap", marginTop: 16 }}>
            <span style={{ fontSize: 14, fontWeight: 500 }}>{t("qb.addA")}</span>
            {TYPES.map(([k]) => <button key={k} type="button" className="btn" style={{ minHeight: 36 }} onClick={() => add(k)}>{label(k)}</button>)}
          </div>
        </div>

        <aside aria-label={t("course.settings")} className={`card ${s.panel}`}>
          <div role="tablist" aria-label={t("qb.settingsSections")} className={s.panelTabs}>
            <button type="button" role="tab" aria-selected={tab === "question"} onClick={() => setTab("question")}>{t("sheet.question", { n: qi + 1 || "" })}</button>
            <button type="button" role="tab" aria-selected={tab === "quiz"} onClick={() => setTab("quiz")}>{t("type.quiz")}</button>
          </div>
          {tab === "question" && q && (
            <div role="tabpanel" aria-label={t("qb.questionSettings")} className={s.panelBody}>
              <div className="field">
                <label htmlFor="qtype">{t("qb.questionType")}</label>
                <select id="qtype" className="select" value={q.type} onChange={(e) => changeType(e.target.value as QuizQuestionType)}>{TYPES.map(([k]) => <option key={k} value={k}>{label(k)}</option>)}</select>
              </div>
              <div className="field">
                <label htmlFor="qpoints">{t("cm.points")}</label>
                <input id="qpoints" className="input" style={{ maxWidth: 140 }} inputMode="decimal" value={q.points} aria-invalid={!!pointsError} onChange={(e) => setQ(qi, (y) => ({ ...y, points: e.target.value }))} onBlur={() => setQ(qi, (y) => ({ ...y, points: normalizePoints(y.points || "0") }))} />
                {pointsError && <p role="alert" className="hint" style={{ color: "var(--danger)" }}>{t("qb.pointsError")}</p>}
              </div>
              {q.type === "multi" && (
                <fieldset style={{ border: 0, padding: 0, margin: 0 }}>
                  <legend style={{ fontSize: 14, fontWeight: 500, marginBottom: 6 }}>{t("qb.scoring")}</legend>
                  <label className="check" style={{ alignItems: "flex-start" }}><input type="radio" name="scoring" checked={!q.partial_credit} onChange={() => setQ(qi, (y) => ({ ...y, partial_credit: false }))} /><span><span style={{ display: "block" }}>{t("qb.allOrNothing")}</span><span className="muted" style={{ fontSize: 13 }}>{t("qb.allOrNothingHint")}</span></span></label>
                  <label className="check" style={{ alignItems: "flex-start" }}><input type="radio" name="scoring" checked={!!q.partial_credit} onChange={() => setQ(qi, (y) => ({ ...y, partial_credit: true }))} /><span><span style={{ display: "block" }}>{t("qb.partialTitle")}</span><span className="muted" style={{ fontSize: 13 }}>{t("qb.partialHint")}</span></span></label>
                </fieldset>
              )}
              {(q.type === "single" || q.type === "multi") && <div className={s.switchRow}><span style={{ fontSize: 14 }}>{t("qb.shuffleOptions")}</span><Switch checked={!!q.shuffle} onChange={(v) => setQ(qi, (y) => ({ ...y, shuffle: v }))} label={t("qb.shuffleOptions")} /></div>}
              <div className={s.row2}>
                <button type="button" className={s.blockBtn} onClick={() => setCfg((c) => { const copy = { ...JSON.parse(JSON.stringify(q)), id: newId() } as QuizQuestion; const list = c.questions.slice(); list.splice(qi + 1, 0, copy); return { ...c, questions: list }; })}><Icon name="copy" size={15} strokeWidth={2} />{t("qb.duplicate")}</button>
                <button type="button" className={s.blockBtn} onClick={() => { setCfg((c) => ({ ...c, questions: c.questions.filter((x) => x.id !== q.id) })); setSel(Math.max(0, qi - 1)); }}><Icon name="trash" size={15} strokeWidth={2} />{t("qb.delete")}</button>
              </div>
            </div>
          )}
          {tab === "question" && !q && <div className={s.panelBody}><p className="muted" style={{ fontSize: 14 }}>{t("qb.addToSet")}</p></div>}
          {tab === "quiz" && (
            <div role="tabpanel" aria-label={t("qb.quizSettings")} className={s.panelBody}>
              <div className={s.switchRow}><span style={{ fontSize: 14 }}>{t("qb.shuffleQuestions")}</span><Switch checked={cfg.shuffle_questions} onChange={(v) => setCfg((c) => ({ ...c, shuffle_questions: v }))} label={t("qb.shuffleQuestions")} /></div>
              <div className={s.switchRow}><span><span style={{ display: "block", fontSize: 14 }}>{t("qb.showCorrect")}</span><span className="muted" style={{ fontSize: 13 }}>{t("qb.afterPublish")}</span></span><Switch checked={cfg.show_correct} onChange={(v) => setCfg((c) => ({ ...c, show_correct: v }))} label={t("qb.showCorrectLabel")} /></div>
              <p className="muted" style={{ fontSize: 13 }}>{t("qb.autoNote")}</p>
              <button type="button" className="btn btn-quiet" style={{ justifyContent: "flex-start", padding: 0 }} aria-expanded={showSettings} onClick={() => setShowSettings((v) => !v)}>{t("qb.deadlineIn")}</button>
              {showSettings && <AssignmentSettings item={item} courseId={courseId} pointsLocked />}
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}
