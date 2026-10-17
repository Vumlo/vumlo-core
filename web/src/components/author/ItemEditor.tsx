"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { useItem } from "@/lib/api/hooks/courses";
import { AssignmentEditor } from "./AssignmentEditor";
import { FormBuilder } from "./FormBuilder";
import { LectureEditor } from "./LectureEditor";
import { QuizBuilder } from "./QuizBuilder";

/** The editor for an item: its kind and assignment type pick the screen. Only the teaching team gets here. */
export function ItemEditor({ courseId, itemId }: { courseId: string; itemId: string }) {
  const router = useRouter();
  const item = useItem(itemId);
  const { staff, loading } = useCourseAccess(courseId);
  useEffect(() => {
    if (!loading && !staff) router.replace(`/courses/${courseId}/items/${itemId}`);
  }, [loading, staff, courseId, itemId, router]);
  if (item.error) return <p className="error-box" role="alert">{item.error.status === 404 ? "This item doesn't exist." : item.error.title}</p>;
  if (!item.data || loading || !staff) return <p className="loading" role="status">Loading…</p>;
  const it = item.data;
  if (it.kind === "text") return <LectureEditor key={it.id} item={it} courseId={courseId} />;
  switch (it.assignment?.type) {
    case "quiz": return <QuizBuilder key={it.id} item={it} courseId={courseId} />;
    case "form": return <FormBuilder key={it.id} item={it} courseId={courseId} />;
    default: return <AssignmentEditor key={it.id} item={it} courseId={courseId} />;
  }
}
