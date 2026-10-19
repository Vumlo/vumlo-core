"use client";

import { translate } from "@/lib/i18n/core";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { Gradebook } from "@/components/course/Gradebook";
import { StudentCourseGrades } from "@/components/course/StudentCourseGrades";
import { useCourseId } from "@/lib/useRoute";

export default function CourseGradesPage() {
  const courseId = useCourseId();
  const { staff, loading } = useCourseAccess(courseId);
  if (loading) return <p className="loading" role="status">{translate("common.loading")}</p>;
  return staff ? <Gradebook courseId={courseId} /> : <StudentCourseGrades courseId={courseId} />;
}
