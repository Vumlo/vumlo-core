"use client";

import Link from "next/link";
import { Icon } from "@/components/Icon";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { LoadFailed, NotFoundScreen } from "@/components/StateScreen";
import { useGradebook, useStudentAttempts, useSubmissions } from "@/lib/api/hooks/assignments";
import { useItem } from "@/lib/api/hooks/courses";
import { useT } from "@/lib/i18n";
import { courseHref } from "@/lib/routes";
import { SubmissionView } from "./SubmissionView";
import s from "./GradeWorkspace.module.css";

/** A client error says the assignment is not there for this person; anything else is a failed load. */
const missing = (status: number | undefined) => status !== undefined && status >= 400 && status < 500 && status !== 401;

/** Grading one assignment on a page of its own: the student named by `studentId` (else the first one) and what they handed in. */
export function GradeWorkspace({ courseId, assignmentId, studentId }: { courseId: string; assignmentId: string; studentId: string }) {
  const t = useT();
  const { staff, loading } = useCourseAccess(courseId);
  const item = useItem(staff && assignmentId ? assignmentId : null);
  const open = item.data?.assignment ? assignmentId : null;
  const book = useGradebook(open ? courseId : null);
  const subs = useSubmissions(open);
  const rows = book.data?.items ?? [];
  const chosen = studentId || rows[0]?.user_id || "";
  const row = rows.find((r) => r.user_id === chosen);
  const sub = subs.data?.find((r) => r.student_id === chosen);
  const known = row ?? sub;
  const attempts = useStudentAttempts(known ? assignmentId : null, chosen);

  if (loading) return <p className="loading" role="status">{t("common.loading")}</p>;
  if (!staff) return <NotFoundScreen />;
  if (item.error) return missing(item.error.status) ? <NotFoundScreen /> : <LoadFailed />;
  if (item.data && !item.data.assignment) return <NotFoundScreen />;
  if (book.error || subs.error) return <LoadFailed />;
  if (!item.data || !book.data || !subs.data) return <p className="loading" role="status">{t("common.loading")}</p>;
  if (!known) return <NotFoundScreen />;
  const name = `${known.name} ${known.surname}`;

  return (
    <div className={s.page}>
      <div className={s.bar}>
        <Link href={courseHref(courseId, "grades")} className={s.back}><Icon name="chevronLeft" size={18} strokeWidth={2} />{t("gw.back")}</Link>
        <h1 className={s.title}>{item.data.title}</h1>
        <h2 id="student-h" className={s.student}>{name}</h2>
      </div>
      <div className={s.body}>
        <div role="region" aria-label={t("gw.submission")} className={s.pane} tabIndex={0}>
          {attempts.error ? <LoadFailed /> : !attempts.data ? <p className="loading" role="status">{t("common.loading")}</p> : <SubmissionView key={chosen} item={item.data} attempts={attempts.data} studentName={name} />}
        </div>
      </div>
    </div>
  );
}
