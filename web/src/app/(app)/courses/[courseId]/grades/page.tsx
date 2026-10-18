"use client";

import { translate } from "@/lib/i18n/core";
import { useParams } from "next/navigation";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { Gradebook } from "@/components/course/Gradebook";
import { StudentCourseGrades } from "@/components/course/StudentCourseGrades";

export default function CourseGradesPage() {
  const { courseId } = useParams<{ courseId: string }>();
  const { staff, loading } = useCourseAccess(courseId);
  if (loading) return <p className="loading" role="status">{translate("common.loading")}</p>;
  return staff ? <Gradebook courseId={courseId} /> : <StudentCourseGrades courseId={courseId} />;
}
