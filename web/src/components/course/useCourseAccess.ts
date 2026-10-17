"use client";

import { useMyCourses } from "@/lib/api/hooks/courses";
import { useMe } from "@/lib/session";

/**
 * Whether the signed-in user teaches this course. It only decides which actions and tabs to show;
 * the server enforces access (ADR-0004).
 */
export function useCourseAccess(courseId: string) {
  const me = useMe();
  const courses = useMyCourses();
  const user = me.data;
  const mine = courses.data?.find((c) => c.id === courseId);
  const owner = !!mine && mine.owner_id === user?.id;
  const staff = user?.role === "admin" || owner || (user?.role === "teacher" && !!mine);
  return { staff, owner, user, loading: !user || !courses.data };
}
