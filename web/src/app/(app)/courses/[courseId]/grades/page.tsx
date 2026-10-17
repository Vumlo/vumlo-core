"use client";

import { useParams } from "next/navigation";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { Gradebook } from "@/components/course/Gradebook";
import { StudentCourseGrades } from "@/components/course/StudentCourseGrades";

export default function CourseGradesPage() {
  const { courseId } = useParams<{ courseId: string }>();
  const { staff, loading } = useCourseAccess(courseId);
  if (loading) return <p className="loading" role="status">Loading…</p>;
  return staff ? <Gradebook courseId={courseId} /> : <StudentCourseGrades courseId={courseId} />;
}
