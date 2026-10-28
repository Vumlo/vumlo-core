"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState, type MouseEvent } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Icon } from "@/components/Icon";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { LoadFailed, NotFoundScreen } from "@/components/StateScreen";
import { useGradebook, useStudentAttempts, useSubmissions } from "@/lib/api/hooks/assignments";
import { useCourse, useItem } from "@/lib/api/hooks/courses";
import { cellState, inWorkspaceFilter, neighbour, type WorkspaceFilter } from "@/lib/grading";
import { useT } from "@/lib/i18n";
import { courseHref, gradeHref } from "@/lib/routes";
import { useUnloadGuard } from "@/lib/useUnloadGuard";
import { GradePanel } from "./GradePanel";
import { StudentRail } from "./StudentRail";
import { SubmissionView } from "./SubmissionView";
import s from "./GradeWorkspace.module.css";

/** A client error says the assignment is not there for this person; anything else is a failed load. */
const missing = (status: number | undefined) => status !== undefined && status >= 400 && status < 500 && status !== 401;

/** Grading one assignment on a page of its own: the student named by `studentId` (else the first one) and what they handed in. */
export function GradeWorkspace({ courseId, assignmentId, studentId }: { courseId: string; assignmentId: string; studentId: string }) {
  const t = useT();
  const router = useRouter();
  const [picked, setPicked] = useState<WorkspaceFilter | null>(null);
  // Whether the panel holds points or a comment that were not saved; moving away then asks first.
  const dirty = useRef(false);
  useUnloadGuard(() => dirty.current);
  const [leaving, setLeaving] = useState<{ href: string; replace: boolean } | null>(null);
  const { staff, loading } = useCourseAccess(courseId);
  const item = useItem(staff && assignmentId ? assignmentId : null);
  const course = useCourse(staff ? courseId : null);
  // An assignment graded by hand has no submissions, so the list of work waiting would always be empty.
  const filter = picked ?? (item.data?.assignment?.type === "no_submission" ? "all" : "grade");
  const open = item.data?.assignment ? assignmentId : null;
  const book = useGradebook(open ? courseId : null);
  const subs = useSubmissions(open);
  const rows = book.data?.items ?? [];
  const chosen = studentId || rows[0]?.user_id || "";
  const row = rows.find((r) => r.user_id === chosen);
  const sub = subs.data?.find((r) => r.student_id === chosen);
  const known = row ?? sub;
  const attempts = useStudentAttempts(known ? assignmentId : null, chosen);
  // After a move to another student the focus goes to their name, so that a screen reader reads it out.
  const seen = useRef(chosen);
  useEffect(() => {
    if (seen.current && seen.current !== chosen) document.getElementById("student-h")?.focus();
    seen.current = chosen;
  }, [chosen]);

  if (loading) return <p className="loading" role="status">{t("common.loading")}</p>;
  if (!staff || !assignmentId) return <NotFoundScreen />;
  if (item.error) return missing(item.error.status) ? <NotFoundScreen /> : <LoadFailed />;
  if (item.data && !item.data.assignment) return <NotFoundScreen />;
  if (book.error || subs.error) return <LoadFailed />;
  if (!item.data || !book.data || !subs.data) return <p className="loading" role="status">{t("common.loading")}</p>;
  if (!known) return <NotFoundScreen />;
  const name = `${known.name} ${known.surname}`;

  const submitted = new Map(subs.data.map((x) => [x.student_id, x]));
  const states = new Map(rows.map((r) => [r.user_id, cellState(r.cells.find((c) => c.assignment_id === assignmentId), submitted.get(r.user_id))]));
  const late = new Set(subs.data.filter((x) => x.latest?.is_late).map((x) => x.student_id));
  // A former student cannot be graded, so only the list of everyone shows them.
  const shown = rows.filter((r) => (filter === "all" || !r.former) && inWorkspaceFilter(states.get(r.user_id) ?? "none", filter));
  const order = rows.map((r) => r.user_id);
  const visible = new Set(shown.map((r) => r.user_id));
  const place = shown.findIndex((r) => r.user_id === chosen);
  const prev = neighbour(order, visible, chosen, -1);
  const next = neighbour(order, visible, chosen, 1);
  const drafts = rows.filter((r) => !r.former && states.get(r.user_id) === "draft").length;
  const col = book.data.columns.find((c) => c.assignment_id === assignmentId);
  // Past the last student of the list the next one is the first student left in it.
  const wrap = shown.find((r) => r.user_id !== chosen)?.user_id ?? null;

  /** Another student's address; asks first while the panel holds unsaved changes. */
  function pick(id: string | null) {
    if (!id || id === chosen) return;
    const href = gradeHref(courseId, assignmentId, id);
    if (dirty.current) setLeaving({ href, replace: true });
    else router.replace(href);
  }
  function back(e: MouseEvent) {
    if (!dirty.current) return;
    e.preventDefault();
    setLeaving({ href: courseHref(courseId, "grades"), replace: false });
  }

  return (
    <div className={s.page}>
      <div className={s.bar}>
        <Link href={courseHref(courseId, "grades")} className={s.back} onClick={back}><Icon name="chevronLeft" size={18} strokeWidth={2} />{t("gw.back")}</Link>
        <h1 className={s.title}>{item.data.title}{course.data && <span className={s.course}>{course.data.title}</span>}</h1>
        <h2 id="student-h" tabIndex={-1} className={s.student}>{name}</h2>
        <div className={s.stepper}>
          <button type="button" className="icon-btn" aria-label={t("gw.prev")} disabled={!prev} onClick={() => pick(prev)}><Icon name="chevronLeft" size={18} strokeWidth={2} /></button>
          <span role="status" className={s.counter}>{t("gw.counter", { n: place >= 0 ? place + 1 : "–", total: shown.length })}</span>
          <button type="button" className="icon-btn" aria-label={t("gw.next")} disabled={!next} onClick={() => pick(next)}><Icon name="chevronRight" size={18} strokeWidth={2} /></button>
        </div>
        <select aria-label={t("gw.student")} className={`select ${s.pick}`} value={chosen} onChange={(e) => pick(e.target.value)}>
          {rows.map((r) => <option key={r.user_id} value={r.user_id}>{r.name} {r.surname}</option>)}
        </select>
        <select aria-label={t("gw.show")} className={`select ${s.pick}`} value={filter} onChange={(e) => setPicked(e.target.value as WorkspaceFilter)}>
          <option value="grade">{t("gw.fGrade")}</option>
          <option value="draft">{t("gw.fDraft")}</option>
          <option value="all">{t("gw.fAll")}</option>
        </select>
      </div>
      {shown.length === 0 && (
        <p className={s.empty} role="status">
          {filter === "draft" ? t("gw.noDrafts") : t("gw.nothingLeft")}
          <Link href={courseHref(courseId, "grades")} className={s.link} onClick={back}>{t("gw.toGradebook")}</Link>
        </p>
      )}
      <div className={s.body}>
        <StudentRail rows={shown} states={states} late={late} selected={chosen} onPick={pick} />
        <div role="region" aria-label={t("gw.submission")} className={s.pane} tabIndex={0}>
          {attempts.error ? <LoadFailed /> : !attempts.data ? <p className="loading" role="status">{t("common.loading")}</p> : <SubmissionView key={chosen} item={item.data} attempts={attempts.data} />}
        </div>
        {row && col && (
          <GradePanel key={chosen} assignmentId={assignmentId} row={row} col={col} cell={row.cells.find((c) => c.assignment_id === assignmentId)} former={!!row.former} drafts={drafts} dirty={dirty} onSavedDraft={() => pick(next ?? wrap)} />
        )}
      </div>
      {leaving && (
        <ConfirmDialog title={t("gb.discardTitle")} confirm={t("gb.discard")} danger onClose={() => setLeaving(null)} onConfirm={() => {
          dirty.current = false;
          if (leaving.replace) router.replace(leaving.href);
          else router.push(leaving.href);
          setLeaving(null);
        }}>
          {t("gb.discardBody")}
        </ConfirmDialog>
      )}
    </div>
  );
}
