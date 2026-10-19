"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { BlockRenderer } from "@/components/doc/BlockRenderer";
import { Icon } from "@/components/Icon";
import { useContentTree, useCoursePeople } from "@/lib/api/hooks/courses";
import { useCourseSubmissions, useGradebook, useGradeHistory, usePublishAllDrafts, useSaveGrade, useStudentAttempts, usePublishGrades } from "@/lib/api/hooks/assignments";
import type { Document, GradeHistoryEntry, GradebookCell, GradebookColumn, GradebookRow } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { allItems } from "@/lib/course";
import { dayLabel, dueLabel, initials, plural, sinceLabel, weekdayDate, timeOf } from "@/lib/format";
import { validScore } from "@/lib/points";
import { CourseHeader } from "./CourseHeader";
import { useT } from "@/lib/i18n";
import s from "./Gradebook.module.css";
import { itemHref } from "@/lib/routes";

/** One recorded change as a line: the server sends the action and the values, the screen words them. */
function historyLine(h: GradeHistoryEntry, t: ReturnType<typeof useT>): string {
  const what =
    h.action === "publish"
      ? h.score ? t("gb.hPublished", { score: h.score }) : h.feedback ? t("gb.hPublishedFeedback") : t("gb.hWithdrawn")
      : h.score ? t("gb.hDraft", { score: h.score }) : h.feedback ? t("gb.hDraftFeedback") : t("gb.hCleared");
  return h.actor ? `${what}, ${h.actor.name} ${h.actor.surname}` : what;
}

type Sel = { studentId: string; assignmentId: string };
type CellState = "pub" | "draft" | "grade" | "none";

function Drawer({ sel, row, col, cell, toGrade, onClose }: { sel: Sel; row: GradebookRow; col: GradebookColumn; cell: GradebookCell | undefined; toGrade: boolean; onClose: () => void }) {
  const t = useT();
  const save = useSaveGrade(sel.assignmentId);
  const publish = usePublishGrades(sel.assignmentId);
  const attempts = useStudentAttempts(sel.assignmentId, sel.studentId);
  const history = useGradeHistory(sel.assignmentId, sel.studentId);
  const [points, setPoints] = useState(cell?.draft_score ?? cell?.published_score ?? "");
  const [comment, setComment] = useState(cell?.draft_feedback ?? cell?.published_feedback ?? "");
  const [open, setOpen] = useState(false);
  const [touched, setTouched] = useState(false);
  const latest = attempts.data?.[0];
  const invalid = points !== "" && !validScore(points, col.max_points);
  const former = !!row.former;
  const content = latest?.content as Document | { answers: Record<string, unknown> } | undefined;

  async function saveDraft() {
    setTouched(true);
    if (invalid) return false;
    return !!(await save.run(sel.studentId, points.trim() === "" ? null : points.trim(), comment));
  }
  async function saveAndPublish() {
    if (!(await saveDraft())) return;
    await publish.run([sel.studentId]);
  }
  const err = save.error ?? publish.error;

  return (
    <aside aria-labelledby="drawer-h" className={`card ${s.drawer}`}>
      <div className="row" style={{ alignItems: "flex-start", justifyContent: "space-between", gap: 8 }}>
        <div>
          <p className="muted" style={{ fontSize: 13 }}>{col.title}</p>
          <h2 id="drawer-h" style={{ fontSize: 19, fontWeight: 600 }}>{row.name} {row.surname}</h2>
        </div>
        <button type="button" className="icon-btn" aria-label={t("common.close")} onClick={onClose}><Icon name="close" size={18} strokeWidth={2} /></button>
      </div>
      {latest ? (
        <div>
          <p style={{ fontSize: 14 }}>{t("gb.submittedAt", { day: weekdayDate(latest.submitted_at), time: timeOf(latest.submitted_at) })}</p>
          <p className="muted" style={{ fontSize: 13 }}>{t("grading.attempt", { n: latest.attempt })}{latest.is_late ? `, ${t("gb.late")}` : `, ${t("gb.onTime")}`}</p>
          <button type="button" className="btn btn-quiet" style={{ padding: 0, minHeight: 36 }} aria-expanded={open} onClick={() => setOpen((o) => !o)}>{open ? t("gb.hide") : t("gb.open")}</button>
          {open && (
            <div className={s.submission}>
              {content && "blocks" in content ? <BlockRenderer doc={content} size={14} /> : <p>{t.plural("gb.answers", Object.keys((content as { answers?: object } | undefined)?.answers ?? {}).length)}</p>}
            </div>
          )}
        </div>
      ) : <p className="muted" style={{ fontSize: 14 }}>{toGrade ? "" : t("gb.nothing")}</p>}
      <div className="stack" style={{ gap: 10 }}>
        <label htmlFor="points" style={{ fontSize: 14, fontWeight: 500 }}>{t("cm.points")}</label>
        <div className="row" style={{ gap: 10 }}>
          <input id="points" className="input" style={{ maxWidth: 120 }} inputMode="decimal" value={points} disabled={former} aria-invalid={touched && invalid} onChange={(e) => { setPoints(e.target.value); setTouched(false); }} />
          <span className="muted" style={{ fontSize: 14 }}>{t("gb.outOf", { max: col.max_points })}</span>
        </div>
        {touched && invalid && <p role="alert" className="hint" style={{ color: "var(--danger)" }}>{t("gb.invalid", { max: col.max_points })}</p>}
        <label htmlFor="comment" style={{ fontSize: 14, fontWeight: 500 }}>{t("gb.commentFor", { name: row.name })}</label>
        <textarea id="comment" className="textarea" style={{ minHeight: 80 }} value={comment} disabled={former} onChange={(e) => setComment(e.target.value)} />
        {err && <p role="alert" className="alert alert-danger">{err.errors[0]?.message ?? err.detail ?? err.title}</p>}
        {former ? <p className="muted" style={{ fontSize: 13 }}>{t("gb.left", { name: row.name })}</p> : (
          <>
            <div className="row" style={{ gap: 10, flexWrap: "wrap" }}>
              <button type="button" className="btn" onClick={() => void saveDraft()} disabled={save.pending}>{t("gb.saveDraft")}</button>
              <button type="button" className="btn btn-primary" onClick={() => void saveAndPublish()} disabled={save.pending || publish.pending}>{t("cm.publish")}</button>
            </div>
            <p className="muted" style={{ fontSize: 13 }}>{t("gb.sees", { name: row.name })}</p>
          </>
        )}
      </div>
      <div>
        <h3 style={{ fontSize: 14, fontWeight: 600, marginBottom: 6 }}>{t("gb.history")}</h3>
        <ol className={s.hist}>
          {(history.data ?? []).map((h) => <li key={h.id}><span>{historyLine(h, t)}</span><span className="muted">{sinceLabel(h.at)}</span></li>)}
          {history.data?.length === 0 && <li className="muted">{t("gb.noChanges")}</li>}
        </ol>
        <p className="muted" style={{ marginTop: 8, fontSize: 12 }}>{t("gb.recorded")}</p>
      </div>
    </aside>
  );
}

export function Gradebook({ courseId }: { courseId: string }) {
  const t = useT();
  const book = useGradebook(courseId);
  const tree = useContentTree(courseId);
  const people = useCoursePeople(courseId);
  const cols = book.data?.columns ?? [];
  const ids = cols.map((c) => c.assignment_id);
  const subs = useCourseSubmissions(courseId, ids);
  const publishAll = usePublishAllDrafts();
  const [q, setQ] = useState("");
  const [group, setGroup] = useState("all");
  const [sel, setSel] = useState<Sel | null>(null);

  const items = tree.data ? allItems(tree.data) : [];
  const groupOf = useMemo(() => new Map((people.data?.people ?? []).map((p) => [p.user_id, p.group ?? t("people.direct")])), [people.data, t]);
  const groups = [...new Set((people.data?.people ?? []).filter((p) => p.role === "student" && !p.former).map((p) => p.group ?? t("people.direct")))];

  const state = (row: GradebookRow, col: GradebookColumn): { st: CellState; cell?: GradebookCell } => {
    const cell = row.cells.find((c) => c.assignment_id === col.assignment_id);
    if (cell && cell.has_unpublished_changes && cell.draft_score !== null && cell.draft_score !== undefined) return { st: "draft", cell };
    if (cell?.published_score) return { st: "pub", cell };
    const sub = subs.data?.[col.assignment_id]?.find((x) => x.student_id === row.user_id);
    if (sub?.status === "submitted" && !(cell && cell.draft_score)) return { st: "grade", cell };
    return { st: "none", cell };
  };

  const rows = (book.data?.items ?? []).filter((r) => `${r.name} ${r.surname}`.toLowerCase().includes(q.trim().toLowerCase()) && (group === "all" || groupOf.get(r.user_id) === group));
  const draftCols = (book.data?.columns ?? []).filter((c) => (book.data?.items ?? []).some((r) => !r.former && r.cells.some((x) => x.assignment_id === c.assignment_id && x.has_unpublished_changes && x.draft_score !== null)));
  const draftCount = (book.data?.items ?? []).filter((r) => !r.former).reduce((a, r) => a + r.cells.filter((x) => x.has_unpublished_changes && x.draft_score !== null).length, 0);

  const note = (col: GradebookColumn) => {
    const item = items.find((i) => i.id === col.assignment_id);
    const due = item?.assignment?.due_at;
    const list = subs.data?.[col.assignment_id] ?? [];
    const submitted = list.filter((x) => x.status === "submitted").length;
    const rowsFor = (book.data?.items ?? []).filter((r) => !r.former);
    const toGrade = rowsFor.filter((r) => state(r, col).st === "grade").length;
    const drafts = rowsFor.filter((r) => state(r, col).st === "draft").length;
    if (item?.assignment?.type === "no_submission") return { text: t("gb.byHand"), tone: "muted" };
    if (toGrade || drafts) return { text: [toGrade ? t("gb.toGradeN", { n: toGrade }) : "", drafts ? plural(drafts, "draft") : ""].filter(Boolean).join(", "), tone: "accent" };
    if (due && new Date(due) >= now() && submitted === 0) return { text: t("gb.dueNone", { day: dayLabel(due) }), tone: "muted" };
    if (due && new Date(due) < now()) return { text: t("gb.closedAll", { day: dayLabel(due) }), tone: "muted" };
    return { text: due ? t("bar.due", { when: dueLabel(due) }) : t("due.none"), tone: "muted" };
  };

  const selRow = sel && book.data?.items.find((r) => r.user_id === sel.studentId);
  const selCol = sel && book.data?.columns.find((c) => c.assignment_id === sel.assignmentId);

  return (
    <div className="container">
      <CourseHeader courseId={courseId} />
      <div className={s.bar}>
        <label className="search" style={{ flex: "0 1 260px", background: "var(--surface)" }}>
          <Icon name="search" size={16} />
          <input type="search" aria-label={t("gb.find")} placeholder={t("gb.find")} value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
        <select aria-label={t("people.group")} className="select" style={{ width: "auto", minHeight: 40 }} value={group} onChange={(e) => setGroup(e.target.value)}>
          <option value="all">{t("gb.allStudents")}</option>
          {groups.map((g) => <option key={g} value={g}>{g.startsWith("CS-") ? t("people.group2", { name: g }) : g}</option>)}
        </select>
        <span style={{ flex: "1 1 auto" }} />
        <button type="button" className="btn btn-primary" disabled={draftCount === 0 || publishAll.pending} onClick={() => void publishAll.run(draftCols.map((c) => c.assignment_id))}>
          {t("gb.publishDrafts", { n: plural(draftCount, "draft") })}
        </button>
      </div>
      {publishAll.error && <p role="alert" className="alert alert-danger" style={{ marginBottom: 12 }}>{publishAll.error.detail ?? publishAll.error.title}</p>}
      <div className={s.legend}>
        <span><strong style={{ color: "var(--text)" }}>18.50</strong> {t("gb.legendPub")}</span>
        <span><span className={s.draft}>22.50</span> {t("gb.legendDraft")}</span>
        <span><span className={s.dot} /> {t("gb.legendSub")}</span>
      </div>
      {!book.data ? <p className="loading" role="status">{t("common.loading")}</p> : (
        <div className={s.body}>
          <div className={`card ${s.gridWrap}`}>
            <div role="grid" aria-label={t("nav.grades")} className={s.grid}>
              <div role="row" className={`${s.row} ${s.head}`}>
                <div role="columnheader" className={s.name} style={{ fontSize: 13, fontWeight: 500, color: "var(--muted)" }}>{t("grading.student")}</div>
                {cols.map((c) => {
                  const n = note(c);
                  return (
                    <div key={c.assignment_id} role="columnheader" className={s.col}>
                      <Link href={itemHref(courseId, c.assignment_id)}>{c.title}</Link>
                      <span className={s.colMeta}>{t("gb.colOut", { max: c.max_points })}</span>
                      <span className={s.colNote} style={{ color: n.tone === "accent" ? "var(--accent-fg)" : undefined }}>{n.text}</span>
                    </div>
                  );
                })}
              </div>
              {rows.map((r) => (
                <div key={r.user_id} role="row" className={s.row}>
                  <div role="rowheader" className={s.name}>
                    <span aria-hidden="true" className={s.avatar}>{initials(r.name, r.surname)}</span>
                    <span>
                      <span style={{ display: "block", fontWeight: 500, color: r.former ? "var(--muted)" : undefined }}>{r.name} {r.surname}</span>
                      {r.former && <span className="muted" style={{ display: "block", fontSize: 12 }}>{t("gb.leftLocked")}</span>}
                    </span>
                  </div>
                  {cols.map((c) => {
                    const { st, cell } = state(r, c);
                    const label = `${r.name} ${r.surname}, ${c.title}, ${st === "pub" ? t("gb.aPub", { score: cell?.published_score ?? "" }) : st === "draft" ? t("gb.aDraft", { score: cell?.draft_score ?? "" }) : st === "grade" ? t("gb.aGrade") : t("gb.aNone")}${r.former ? `, ${t("gb.aLocked")}` : ""}`;
                    const selected = sel?.studentId === r.user_id && sel.assignmentId === c.assignment_id;
                    return (
                      <div key={c.assignment_id} role="gridcell" className={s.cell}>
                        <button type="button" className={s.cellBtn} aria-label={label} aria-pressed={selected} onClick={() => setSel({ studentId: r.user_id, assignmentId: c.assignment_id })}>
                          {st === "pub" && <span className={s.pub}>{cell?.published_score}</span>}
                          {st === "draft" && <span className={s.draft}>{cell?.draft_score}</span>}
                          {st === "grade" && <><span className={s.dot} /><span style={{ fontSize: 13 }}>{t("gb.toGrade")}</span></>}
                          {st === "none" && <span aria-hidden="true" className="muted">—</span>}
                        </button>
                      </div>
                    );
                  })}
                </div>
              ))}
              {rows.length === 0 && <p className="empty">{t("people.noStudents")}</p>}
            </div>
          </div>
          {sel && selRow && selCol && (
            <Drawer key={`${sel.studentId}:${sel.assignmentId}`} sel={sel} row={selRow} col={selCol} cell={selRow.cells.find((c) => c.assignment_id === sel.assignmentId)} toGrade={state(selRow, selCol).st === "grade"} onClose={() => setSel(null)} />
          )}
        </div>
      )}
    </div>
  );
}
