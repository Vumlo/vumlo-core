"use client";

import { useCourseAccess } from "@/components/course/useCourseAccess";
import { LoadFailed, NotFoundScreen } from "@/components/StateScreen";
import { useItem } from "@/lib/api/hooks/courses";
import { isLocked } from "@/lib/course";
import { useT } from "@/lib/i18n";
import { AssignmentPage } from "./AssignmentPage";
import { FileAssignmentPage } from "./FileAssignmentPage";
import { FilePage } from "./FilePage";
import { FormPage } from "./FormPage";
import { LecturePage } from "./LecturePage";
import { LockedPage } from "./LockedPage";
import { NoSubmissionPage } from "./NoSubmissionPage";
import { QuizPage } from "./QuizPage";

/** One URL for every kind of item; the item's kind and assignment type pick the screen. */
export function ItemView({ courseId, itemId }: { courseId: string; itemId: string }) {
  const t = useT();
  const item = useItem(itemId);
  const { staff, loading } = useCourseAccess(courseId);
  if (item.error) return item.error.status === 404 || item.error.status === 403 ? <NotFoundScreen /> : <LoadFailed title={item.error.title} />;
  if (!item.data || loading) return <p className="loading" role="status">{t("common.loading")}</p>;
  const it = item.data;
  // The server sends a locked item as its title only; staff are never locked (ADR-0092).
  if (!staff && isLocked(it)) return <LockedPage item={it} courseId={courseId} />;
  if (it.kind === "text") return <LecturePage item={it} courseId={courseId} staff={staff} />;
  if (it.kind === "file") return <FilePage item={it} courseId={courseId} staff={staff} />;
  switch (it.assignment?.type) {
    case "no_submission": return <NoSubmissionPage item={it} courseId={courseId} staff={staff} />;
    case "quiz": return <QuizPage item={it} courseId={courseId} staff={staff} />;
    case "form": return <FormPage item={it} courseId={courseId} staff={staff} />;
    case "file": return <FileAssignmentPage item={it} courseId={courseId} staff={staff} />;
    default: return <AssignmentPage item={it} courseId={courseId} staff={staff} />;
  }
}
