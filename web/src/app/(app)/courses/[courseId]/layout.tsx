"use client";

import { use, type ReactNode } from "react";
import { LoadFailed, NotFoundScreen } from "@/components/StateScreen";
import { useCourse } from "@/lib/api/hooks/courses";
import { useT } from "@/lib/i18n";

/** Every screen of a course: when the course is missing or not visible to the person, the whole page is "not found". */
export default function CourseLayout({ children, params }: { children: ReactNode; params: Promise<{ courseId: string }> }) {
  const t = useT();
  const { courseId } = use(params);
  const course = useCourse(courseId);
  const status = course.error?.status;
  if (status === 404 || status === 403) return <NotFoundScreen />;
  if (course.error && status !== 401) return <LoadFailed title={t("error.loadFailed")} />;
  return <>{children}</>;
}
