"use client";

import { useCourseAccess } from "@/components/course/useCourseAccess";
import { LoadFailed, NotFoundScreen } from "@/components/StateScreen";
import { useItem } from "@/lib/api/hooks/courses";
import { useT } from "@/lib/i18n";
import { AssignmentPage } from "./AssignmentPage";
import { FormPage } from "./FormPage";
import { LecturePage } from "./LecturePage";
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
  if (it.kind === "text") return <LecturePage item={it} courseId={courseId} staff={staff} />;
  switch (it.assignment?.type) {
    case "no_submission": return <NoSubmissionPage item={it} courseId={courseId} staff={staff} />;
    case "quiz": return <QuizPage item={it} courseId={courseId} staff={staff} />;
    case "form": return <FormPage item={it} courseId={courseId} staff={staff} />;
    default: return <AssignmentPage item={it} courseId={courseId} staff={staff} />;
  }
}
