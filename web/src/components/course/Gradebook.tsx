"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Icon } from "@/components/Icon";
import { errorText } from "@/lib/api/client";
import { useContentTree, useCoursePeople } from "@/lib/api/hooks/courses";
import { useCourseSubmissions, useGradebook, usePublishAllDrafts } from "@/lib/api/hooks/assignments";
import type { GradebookColumn, GradebookRow } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { allItems } from "@/lib/course";
import { dayLabel, dueLabel, initials, plural } from "@/lib/format";
import { cellState, matchesStatus, type StatusFilter } from "@/lib/grading";
import { CourseHeader } from "./CourseHeader";
import { useT } from "@/lib/i18n";
import s from "./Gradebook.module.css";
import { gradeHref, itemHref } from "@/lib/routes";

export function Gradebook({ courseId }: { courseId: string }) {
  const t = useT();
  const router = useRouter();
  const book = useGradebook(courseId);
  const tree = useContentTree(courseId);
  const people = useCoursePeople(courseId);
  const cols = book.data?.columns ?? [];
  const ids = cols.map((c) => c.assignment_id);
  const subs = useCourseSubmissions(courseId, ids);
  const publishAll = usePublishAllDrafts();
  const [q, setQ] = useState("");
  const [group, setGroup] = useState("all");
  const [pick, setPick] = useState("all");
  const [status, setStatus] = useState<StatusFilter>("all");
  const [lateOnly, setLateOnly] = useState(false);
  const [asking, setAsking] = useState(false);

  const items = tree.data ? allItems(tree.data) : [];
  const groupOf = useMemo(() => new Map((people.data?.people ?? []).map((p) => [p.user_id, p.group ?? t("people.direct")])), [people.data, t]);
  const groups = [...new Set((people.data?.people ?? []).filter((p) => p.role === "student" && !p.former).map((p) => p.group ?? t("people.direct")))];

  const state = (row: GradebookRow, col: GradebookColumn) => {
    const cell = row.cells.find((c) => c.assignment_id === col.assignment_id);
    const sub = subs.data?.[col.assignment_id]?.find((x) => x.student_id === row.user_id);
    return { st: cellState(cell, sub), cell, late: !!sub?.latest?.is_late };
  };

  const visibleCols = pick === "all" ? cols : cols.filter((c) => c.assignment_id === pick);
  // A former student cannot be graded, so a status other than "all" leaves them out, as the counts above do.
  const rows = (book.data?.items ?? []).filter((r) => `${r.name} ${r.surname}`.toLowerCase().includes(q.trim().toLowerCase()) && (group === "all" || groupOf.get(r.user_id) === group)
    && (status === "all" || !r.former)
    && matchesStatus(visibleCols.map((c) => { const { st, late } = state(r, c); return { state: st, late }; }), { status, late: lateOnly }));
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
          {groups.map((g) => <option key={g} value={g}>{g}</option>)}
        </select>
        <select aria-label={t("gb.filterAssignment")} className="select" style={{ width: "auto", minHeight: 40 }} value={pick} onChange={(e) => setPick(e.target.value)}>
          <option value="all">{t("gb.allAssignments")}</option>
          {cols.map((c) => <option key={c.assignment_id} value={c.assignment_id}>{c.title}</option>)}
        </select>
        <select aria-label={t("gb.filterStatus")} className="select" style={{ width: "auto", minHeight: 40 }} value={status} onChange={(e) => setStatus(e.target.value as StatusFilter)}>
          <option value="all">{t("gb.statusAll")}</option>
          <option value="grade">{t("gb.toGrade")}</option>
          <option value="draft">{t("gb.statusDraft")}</option>
          <option value="pub">{t("gb.statusPublished")}</option>
          <option value="none">{t("gb.statusNone")}</option>
        </select>
        <label className="check"><input type="checkbox" checked={lateOnly} onChange={(e) => setLateOnly(e.target.checked)} />{t("gb.lateOnly")}</label>
        <span style={{ flex: "1 1 auto" }} />
        <div className={s.legend}>
          <span><strong style={{ color: "var(--text)" }}>18.50</strong> {t("gb.legendPub")}</span>
          <span><span className={s.draft}>22.50</span> {t("gb.legendDraft")}</span>
          <span><span className={s.dot} /> {t("gb.legendSub")}</span>
        </div>
        <button type="button" className="btn btn-primary" disabled={draftCount === 0 || publishAll.pending} onClick={() => setAsking(true)}>
          {t("gb.publishDrafts", { n: plural(draftCount, "draft") })}
        </button>
      </div>
      {publishAll.error && <p role="alert" className="alert alert-danger" style={{ marginBottom: 12 }}>{errorText(publishAll.error)}</p>}
      {asking && (
        <ConfirmDialog title={t("gb.confirmTitle")} confirm={t("cm.publish")} pending={publishAll.pending} onClose={() => setAsking(false)} onConfirm={async () => { await publishAll.run(draftCols.map((c) => c.assignment_id)); setAsking(false); }}>
          {t("gb.confirmBody", { n: plural(draftCount, "draft") })}
        </ConfirmDialog>
      )}
      {!book.data ? <p className="loading" role="status">{t("common.loading")}</p> : (
        <div className={s.body}>
          <div className={`card ${s.gridWrap}`}>
            <div role="grid" aria-label={t("nav.grades")} className={s.grid}>
              <div role="row" className={`${s.row} ${s.head}`}>
                <div role="columnheader" className={s.name} style={{ fontSize: 13, fontWeight: 500, color: "var(--muted)" }}>{t("grading.student")}</div>
                {visibleCols.map((c) => {
                  const n = note(c);
                  return (
                    <div key={c.assignment_id} role="columnheader" className={s.col}>
                      <Link href={itemHref(courseId, c.assignment_id)}>{c.title}</Link>
                      <span className={s.colMeta}>{t("gb.colOut", { max: c.max_points })}</span>
                      <span className={s.colNote} data-tone={n.tone}>{n.text}</span>
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
                  {visibleCols.map((c) => {
                    const { st, cell } = state(r, c);
                    const label = `${r.name} ${r.surname}, ${c.title}, ${st === "pub" ? t("gb.aPub", { score: cell?.published_score ?? "" }) : st === "draft" ? t("gb.aDraft", { score: cell?.draft_score ?? "" }) : st === "grade" ? t("gb.aGrade") : t("gb.aNone")}${r.former ? `, ${t("gb.aLocked")}` : ""}`;
                    return (
                      <div key={c.assignment_id} role="gridcell" className={s.cell}>
                        <button type="button" className={s.cellBtn} aria-label={label} onClick={() => router.push(gradeHref(courseId, c.assignment_id, r.user_id))}>
                          {st === "pub" && <span className={s.pub}>{cell?.published_score}</span>}
                          {st === "draft" && <span className={s.draft}>{cell?.draft_score}</span>}
                          {st === "grade" && <><span className={s.dot} /><span style={{ fontSize: 13 }}>{t("gb.toGrade")}</span></>}
                          {st === "none" && <span aria-hidden="true" className={s.none} />}
                        </button>
                      </div>
                    );
                  })}
                </div>
              ))}
              {rows.length === 0 && <p className="empty">{t("people.noStudents")}</p>}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
