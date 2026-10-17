"use client";

import { useParams, useRouter } from "next/navigation";
import { useEffect } from "react";
import { CourseSettings } from "@/components/course/CourseSettings";
import { useCourseAccess } from "@/components/course/useCourseAccess";

export default function SettingsPage() {
  const { courseId } = useParams<{ courseId: string }>();
  const router = useRouter();
  const { staff, loading } = useCourseAccess(courseId);
  useEffect(() => {
    if (!loading && !staff) router.replace(`/courses/${courseId}`);
  }, [loading, staff, courseId, router]);
  return staff ? <CourseSettings courseId={courseId} /> : null;
}
