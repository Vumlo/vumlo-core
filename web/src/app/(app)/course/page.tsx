"use client";

import { translate } from "@/lib/i18n/core";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { StudentContent } from "@/components/course/StudentContent";
import { ContentManage } from "@/components/course/ContentManage";
import { useCourseId } from "@/lib/useRoute";

export default function CoursePage() {
  const courseId = useCourseId();
  const { can, staff, loading } = useCourseAccess(courseId);
  if (loading) return <p className="loading" role="status">{translate("common.loading")}</p>;
  // Anyone with an ability sees the course as its teachers do; what they may change follows `edit_content`.
  return staff ? <ContentManage courseId={courseId} canEdit={can.edit_content} /> : <StudentContent courseId={courseId} />;
}
