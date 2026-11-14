"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { CourseSettings } from "@/components/course/CourseSettings";
import { useCourseAccess } from "@/components/course/useCourseAccess";
import { courseHref } from "@/lib/routes";
import { useCourseId } from "@/lib/useRoute";

export default function SettingsPage() {
  const courseId = useCourseId();
  const router = useRouter();
  const { settings: allowed, loading } = useCourseAccess(courseId);
  useEffect(() => {
    if (!loading && !allowed) router.replace(courseHref(courseId));
  }, [loading, allowed, courseId, router]);
  return allowed ? <CourseSettings courseId={courseId} /> : null;
}
