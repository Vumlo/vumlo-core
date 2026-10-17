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
import s from "./Author.module.css";

const TYPES: [QuizQuestionType, string][] = [["single", "Single choice"], ["multi", "Multiple choice"], ["number", "Number"], ["short", "Short text"], ["long", "Long answer"]];
const label = (t: QuizQuestionType) => TYPES.find(([k]) => k === t)![1];
const short = (t: string) => (t.length > 28 ? `${t.slice(0, 26).trimEnd()}…` : t || "New question");

function blank(type: QuizQuestionType): QuizQuestion {
  const opts = type === "single" || type === "multi" ? [{ id: newId(), text: "" }, { id: newId(), text: "" }] : [];
  return { id: newId(), type, prompt: "", points: "1.00", options: opts, correct: [], ...(type === "number" ? { answer: "", tolerance: "0" } : {}) };
}

function summary(q: QuizQuestion): string {
  if (q.type === "single" || q.type === "multi") {
    const names = q.options.filter((o) => q.correct?.includes(o.id)).map((o) => o.text || "empty");
    return `Correct: ${names.join(", ") || "none chosen"}${q.type === "multi" && q.partial_credit ? ". Partial credit." : ""}`;
  }
  if (q.type === "number") return `Correct: ${q.answer || "not set"}`;
  return "You grade this one yourself.";
}

export function QuizBuilder({ item, courseId }: { item: Item; courseId: string }) {
  const update = useUpdateItem(courseId);
  const publish = usePublishItem(courseId);
  const [title, setTitle] = useState(item.title);
  const [cfg, setCfg] = useState<QuizConfig>(() => (item.assignment!.config as unknown as QuizConfig).questions ? (item.assignment!.config as unknown as QuizConfig) : { questions: [], shuffle_questions: false, show_correct: true });
  const [sel, setSel] = useState(0);
  const [tab, setTab] = useState<"question" | "quiz">("question");
  const [showSettings, setShowSettings] = useState(false);

  const total = useMemo(() => sumPoints(cfg.questions.map((q) => (Number.isNaN(toCents(q.points)) ? "0" : q.points))), [cfg.questions]);
  const draft = useMemo(() => ({ title, cfg, total }), [title, cfg, total]);
  const auto = useAutosave(draft, (d) => update.run(item.id, { title: d.title.trim() || item.title, config: d.cfg as unknown as Record<string, unknown>, ...(toCents(d.total) > 0 ? { max_points: d.total } : {}) }));
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
      <AuthorBar courseId={courseId} itemId={item.id} title={title} chip={isDraft ? { text: "Draft, students can't see it", tone: "draft" } : { text: "Published", tone: "ok" }} saved={auto.savedAt} pending={auto.pending} failed={auto.failed}>
        <Link href={`/courses/${courseId}/items/${item.id}`} className="btn">Preview as student</Link>
        {isDraft && <button type="button" className="btn btn-primary" disabled={publish.pending} onClick={async () => { await auto.flush(); await publish.run(item.id); }}>Publish quiz</button>}
      </AuthorBar>
      <div className={s.shell}>
        <nav aria-label="Questions" className={`card ${s.qnav}`}>
          <div className="row" style={{ justifyContent: "space-between" }}><h2 style={{ fontSize: 15, fontWeight: 600 }}>Questions</h2><span className="muted" style={{ fontSize: 13 }}>{total} pts</span></div>
          <ol>
            {cfg.questions.map((x, i) => (
              <li key={x.id}>
                <button type="button" className={s.qnavBtn} aria-current={i === qi ? "step" : undefined} onClick={() => { setSel(i); setTab("question"); }}>
                  <span className={s.qnum}>{i + 1}</span><span className={s.qshort}>{short(x.prompt)}</span><span className="muted" style={{ fontSize: 12 }}>{x.points}</span>
                </button>
              </li>
            ))}
          </ol>
          <button type="button" className="btn btn-block" onClick={() => add("single")}><Icon name="plus" size={15} strokeWidth={2.2} />Add question</button>
          <p className="muted" style={{ marginTop: 10, fontSize: 13 }}>Questions add up to {total} points, which is what the quiz is worth.</p>
        </nav>

        <div className={s.canvas}>
          <p className="muted" style={{ fontSize: 14, marginBottom: 8 }}><strong style={{ color: "var(--asg-fg)" }}>Assignment</strong> Quiz{a.due_at ? `, due ${dueLabel(a.due_at)}` : ""}, {total} points</p>
          <label htmlFor="qtitle" className="sr-only">Quiz title</label>
          <input id="qtitle" className={s.titleInput} value={title} onChange={(e) => setTitle(e.target.value)} />
          <div className="stack" style={{ gap: 14, marginTop: 16 }}>
            {cfg.questions.map((x, i) => {
              const selected = i === qi;
              const choice = x.type === "single" || x.type === "multi";
              return (
                <article key={x.id} aria-label={`Question ${i + 1}`} className={s.card} data-selected={selected} onClick={() => { if (!selected) setSel(i); }}>
                  <div className={s.cardHead}><span className={s.qnum}>{i + 1}</span><strong>{label(x.type)}</strong><span style={{ marginLeft: "auto" }}>{x.points} pts</span></div>
                  {!selected ? (
                    <>
                      <p style={{ fontSize: 15, fontWeight: 500 }}>{x.prompt || <span className="muted">New question</span>}</p>
                      <p className="muted" style={{ fontSize: 13 }}>{summary(x)}</p>
                    </>
                  ) : (
                    <div className="stack" style={{ gap: 12 }}>
                      <div className="field">
                        <label htmlFor={`prompt-${x.id}`}>Question</label>
                        <textarea id={`prompt-${x.id}`} className="textarea" style={{ minHeight: 70 }} value={x.prompt} onChange={(e) => setQ(i, (y) => ({ ...y, prompt: e.target.value }))} />
                      </div>
                      {choice && (
                        <div>
                          <p style={{ fontSize: 14, fontWeight: 500, marginBottom: 8 }}>{x.type === "multi" ? "Options, tick every correct one" : "Options, pick the correct one"}</p>
                          <div role="group" aria-label="Options" className="stack" style={{ gap: 8 }}>
                            {x.options.map((o, oi) => {
                              const correct = (x.correct ?? []).includes(o.id);
                              return (
                                <div key={o.id} className={s.optRow} data-correct={correct}>
                                  <button type="button" role={x.type === "multi" ? "checkbox" : "radio"} aria-checked={correct} aria-label={`Mark option ${oi + 1} as correct`} className={s.mark} data-shape={x.type === "multi" ? "check" : "radio"} onClick={() => setQ(i, (y) => ({ ...y, correct: toggleCorrect(y, o.id) }))}>
                                    {correct && <Icon name="check" size={13} strokeWidth={3} />}
                                  </button>
                                  <input type="text" aria-label={`Option ${oi + 1}`} value={o.text} placeholder={`Option ${oi + 1}`} onChange={(e) => setQ(i, (y) => ({ ...y, options: y.options.map((z) => (z.id === o.id ? { ...z, text: e.target.value } : z)) }))} />
                                  {correct && <span style={{ fontSize: 12, fontWeight: 600, color: "var(--ok-fg)" }}>Correct</span>}
                                  <button type="button" className="icon-btn" style={{ width: 32, height: 32 }} aria-label={`Remove option ${oi + 1}`} disabled={x.options.length <= 2} onClick={() => setQ(i, (y) => ({ ...y, options: y.options.filter((z) => z.id !== o.id), correct: (y.correct ?? []).filter((c) => c !== o.id) }))}><Icon name="close" size={15} strokeWidth={2} /></button>
                                </div>
                              );
                            })}
                          </div>
                          <button type="button" className="btn" style={{ marginTop: 8, minHeight: 36 }} disabled={x.options.length >= 10} onClick={() => setQ(i, (y) => ({ ...y, options: [...y.options, { id: newId(), text: "" }] }))}><Icon name="plus" size={15} strokeWidth={2.2} />Add option</button>
                        </div>
                      )}
                      {x.type === "number" && (
                        <div className="row" style={{ gap: 14, flexWrap: "wrap" }}>
                          <div className="field"><label htmlFor={`ans-${x.id}`}>Correct answer</label><input id={`ans-${x.id}`} className="input" style={{ maxWidth: 160 }} inputMode="decimal" value={x.answer ?? ""} onChange={(e) => setQ(i, (y) => ({ ...y, answer: e.target.value }))} /></div>
                          <div className="field"><label htmlFor={`tol-${x.id}`}>Accept within</label><input id={`tol-${x.id}`} className="input" style={{ maxWidth: 120 }} inputMode="decimal" value={x.tolerance ?? "0"} onChange={(e) => setQ(i, (y) => ({ ...y, tolerance: e.target.value }))} /></div>
                        </div>
                      )}
                      {(x.type === "long" || x.type === "short") && (
                        <div className="field">
                          <label htmlFor={`notes-${x.id}`}>Notes for grading, only staff see them</label>
                          <textarea id={`notes-${x.id}`} className="textarea" style={{ minHeight: 70 }} value={x.grading_notes ?? ""} onChange={(e) => setQ(i, (y) => ({ ...y, grading_notes: e.target.value }))} />
                          <p className="hint">You grade this answer yourself in the gradebook.</p>
                        </div>
                      )}
                    </div>
                  )}
                </article>
              );
            })}
            {cfg.questions.length === 0 && <p className="empty">This quiz has no questions yet. Add one below.</p>}
          </div>
          <div className="row" style={{ gap: 8, flexWrap: "wrap", marginTop: 16 }}>
            <span style={{ fontSize: 14, fontWeight: 500 }}>Add a question</span>
            {TYPES.map(([t, l]) => <button key={t} type="button" className="btn" style={{ minHeight: 36 }} onClick={() => add(t)}>{l}</button>)}
          </div>
        </div>

        <aside aria-label="Settings" className={`card ${s.panel}`}>
          <div role="tablist" aria-label="Settings sections" className={s.panelTabs}>
            <button type="button" role="tab" aria-selected={tab === "question"} onClick={() => setTab("question")}>Question {qi + 1 || ""}</button>
            <button type="button" role="tab" aria-selected={tab === "quiz"} onClick={() => setTab("quiz")}>Quiz</button>
          </div>
          {tab === "question" && q && (
            <div role="tabpanel" aria-label="Question settings" className={s.panelBody}>
              <div className="field">
                <label htmlFor="qtype">Question type</label>
                <select id="qtype" className="select" value={q.type} onChange={(e) => changeType(e.target.value as QuizQuestionType)}>{TYPES.map(([k, l]) => <option key={k} value={k}>{l}</option>)}</select>
              </div>
              <div className="field">
                <label htmlFor="qpoints">Points</label>
                <input id="qpoints" className="input" style={{ maxWidth: 140 }} inputMode="decimal" value={q.points} aria-invalid={!!pointsError} onChange={(e) => setQ(qi, (y) => ({ ...y, points: e.target.value }))} onBlur={() => setQ(qi, (y) => ({ ...y, points: normalizePoints(y.points || "0") }))} />
                {pointsError && <p role="alert" className="hint" style={{ color: "var(--danger)" }}>Use a number with at most two decimals.</p>}
              </div>
              {q.type === "multi" && (
                <fieldset style={{ border: 0, padding: 0, margin: 0 }}>
                  <legend style={{ fontSize: 14, fontWeight: 500, marginBottom: 6 }}>Scoring</legend>
                  <label className="check" style={{ alignItems: "flex-start" }}><input type="radio" name="scoring" checked={!q.partial_credit} onChange={() => setQ(qi, (y) => ({ ...y, partial_credit: false }))} /><span><span style={{ display: "block" }}>All or nothing</span><span className="muted" style={{ fontSize: 13 }}>Full points only for exactly the right options.</span></span></label>
                  <label className="check" style={{ alignItems: "flex-start" }}><input type="radio" name="scoring" checked={!!q.partial_credit} onChange={() => setQ(qi, (y) => ({ ...y, partial_credit: true }))} /><span><span style={{ display: "block" }}>Partial credit</span><span className="muted" style={{ fontSize: 13 }}>Each right choice adds a share, each wrong one takes a share away, never below zero.</span></span></label>
                </fieldset>
              )}
              {(q.type === "single" || q.type === "multi") && <div className={s.switchRow}><span style={{ fontSize: 14 }}>Shuffle options</span><Switch checked={!!q.shuffle} onChange={(v) => setQ(qi, (y) => ({ ...y, shuffle: v }))} label="Shuffle options" /></div>}
              <div className={s.row2}>
                <button type="button" className={s.blockBtn} onClick={() => setCfg((c) => { const copy = { ...JSON.parse(JSON.stringify(q)), id: newId() } as QuizQuestion; const list = c.questions.slice(); list.splice(qi + 1, 0, copy); return { ...c, questions: list }; })}><Icon name="copy" size={15} strokeWidth={2} />Duplicate question</button>
                <button type="button" className={s.blockBtn} onClick={() => { setCfg((c) => ({ ...c, questions: c.questions.filter((x) => x.id !== q.id) })); setSel(Math.max(0, qi - 1)); }}><Icon name="trash" size={15} strokeWidth={2} />Delete question</button>
              </div>
            </div>
          )}
          {tab === "question" && !q && <div className={s.panelBody}><p className="muted" style={{ fontSize: 14 }}>Add a question to set its type and points.</p></div>}
          {tab === "quiz" && (
            <div role="tabpanel" aria-label="Quiz settings" className={s.panelBody}>
              <div className={s.switchRow}><span style={{ fontSize: 14 }}>Shuffle question order</span><Switch checked={cfg.shuffle_questions} onChange={(v) => setCfg((c) => ({ ...c, shuffle_questions: v }))} label="Shuffle question order" /></div>
              <div className={s.switchRow}><span><span style={{ display: "block", fontSize: 14 }}>Show correct answers</span><span className="muted" style={{ fontSize: 13 }}>After you publish the grades</span></span><Switch checked={cfg.show_correct} onChange={(v) => setCfg((c) => ({ ...c, show_correct: v }))} label="Show correct answers after grades are published" /></div>
              <p className="muted" style={{ fontSize: 13 }}>Choice and number questions are scored automatically as drafts. Long answers wait for you. Nothing reaches students until you publish grades.</p>
              <button type="button" className="btn btn-quiet" style={{ justifyContent: "flex-start", padding: 0 }} aria-expanded={showSettings} onClick={() => setShowSettings((v) => !v)}>Deadline and attempts are in the assignment settings</button>
              {showSettings && <AssignmentSettings item={item} courseId={courseId} pointsLocked />}
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}
