"use client";

import { useParams } from "next/navigation";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { StudentContent } from "@/components/course/StudentContent";
import { ContentManage } from "@/components/course/ContentManage";

export default function CoursePage() {
  const { courseId } = useParams<{ courseId: string }>();
  const { staff, loading } = useCourseAccess(courseId);
  if (loading) return <p className="loading" role="status">Loading…</p>;
  return staff ? <ContentManage courseId={courseId} /> : <StudentContent courseId={courseId} />;
}
