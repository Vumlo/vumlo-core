"use client";

import { useCourse } from "@/lib/api/hooks/courses";
import type { CourseCan } from "@/lib/api/types";

const NOTHING: CourseCan = { view_grades: false, edit_content: false, grade: false, manage_roster: false, manage_course: false };

/**
 * What the signed-in user may do in this course: the server's `can`, which also covers someone who reaches
 * the course through a unit (ADR-0094). `staff` is any ability at all, the split between a student's screen
 * and the teaching side's. It only decides what to show; the server enforces access (ADR-0004).
 */
export function useCourseAccess(courseId: string) {
  const course = useCourse(courseId);
  const can = course.data?.can ?? NOTHING;
  return { can, staff: Object.values(can).some(Boolean), loading: !course.data && !course.error };
}
