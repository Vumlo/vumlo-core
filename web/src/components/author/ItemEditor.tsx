"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { LoadFailed, NotFoundScreen } from "@/components/StateScreen";
import { useItem } from "@/lib/api/hooks/courses";
import { useT } from "@/lib/i18n";
import { AssignmentEditor } from "./AssignmentEditor";
import { FormBuilder } from "./FormBuilder";
import { LectureEditor } from "./LectureEditor";
import { QuizBuilder } from "./QuizBuilder";
import { itemHref } from "@/lib/routes";

/** The editor for an item: its kind and assignment type pick the screen. Only the teaching team gets here. */
export function ItemEditor({ courseId, itemId }: { courseId: string; itemId: string }) {
  const router = useRouter();
  const t = useT();
  const item = useItem(itemId);
  const { staff, loading } = useCourseAccess(courseId);
  useEffect(() => {
    if (!loading && !staff) router.replace(itemHref(courseId, itemId));
  }, [loading, staff, courseId, itemId, router]);
  if (item.error) return item.error.status === 404 || item.error.status === 403 ? <NotFoundScreen /> : <LoadFailed title={item.error.title} />;
  if (!item.data || loading || !staff) return <p className="loading" role="status">{t("common.loading")}</p>;
  const it = item.data;
  if (it.kind === "text") return <LectureEditor key={it.id} item={it} courseId={courseId} />;
  switch (it.assignment?.type) {
    case "quiz": return <QuizBuilder key={it.id} item={it} courseId={courseId} />;
    case "form": return <FormBuilder key={it.id} item={it} courseId={courseId} />;
    default: return <AssignmentEditor key={it.id} item={it} courseId={courseId} />;
  }
}
