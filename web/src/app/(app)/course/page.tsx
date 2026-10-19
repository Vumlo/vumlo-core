"use client";

import { translate } from "@/lib/i18n/core";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { StudentContent } from "@/components/course/StudentContent";
import { ContentManage } from "@/components/course/ContentManage";
import { useCourseId } from "@/lib/useRoute";

export default function CoursePage() {
  const courseId = useCourseId();
  const { staff, loading } = useCourseAccess(courseId);
  if (loading) return <p className="loading" role="status">{translate("common.loading")}</p>;
  return staff ? <ContentManage courseId={courseId} /> : <StudentContent courseId={courseId} />;
}
