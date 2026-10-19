"use client";

import { Suspense, type ReactNode } from "react";
import { LoadFailed, NotFoundScreen } from "@/components/StateScreen";
import { useCourse } from "@/lib/api/hooks/courses";
import { useT } from "@/lib/i18n";
import { useCourseId } from "@/lib/useRoute";

function Course({ courseId, children }: { courseId: string; children: ReactNode }) {
  const t = useT();
  const course = useCourse(courseId);
  const status = course.error?.status;
  if (status === 404 || status === 403) return <NotFoundScreen />;
  if (course.error && status !== 401) return <LoadFailed title={t("error.loadFailed")} />;
  return <>{children}</>;
}

function Guard({ children }: { children: ReactNode }) {
  const courseId = useCourseId();
  return courseId ? <Course courseId={courseId}>{children}</Course> : <NotFoundScreen />;
}

/**
 * Every screen of a course, at `/course/...?id=<course>`: when the course is missing or not visible to the person, the
 * whole page is "not found". The boundary is what a static export needs around `useSearchParams` (ADR-0057).
 */
export default function CourseLayout({ children }: { children: ReactNode }) {
  return (
    <Suspense fallback={null}>
      <Guard>{children}</Guard>
    </Suspense>
  );
}
