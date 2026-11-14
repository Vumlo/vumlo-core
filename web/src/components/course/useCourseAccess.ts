"use client";

import { useCourse, useMyCourses } from "@/lib/api/hooks/courses";
import type { CourseCan } from "@/lib/api/types";
import { useMe } from "@/lib/session";

const NOTHING: CourseCan = { view_grades: false, edit_content: false, grade: false, manage_roster: false, manage_course: false };

/**
 * What the signed-in user may do in this course: the server's `can`, which also covers someone who reaches
 * the course through a unit (ADR-0094). `staff` is any ability at all, the split between a student's screen
 * and the teaching side's. It only decides what to show; the server enforces access (ADR-0004).
 *
 * `manageRoot` is the owner, a teacher of the course, or an administrator: whoever the server lets restore,
 * delete or hand over the course. `can` has no field for those actions (an archived course reads as `false` for
 * every write, and restoring is not one of them), so the Settings tab of an ARCHIVED course opens through it
 * (`settings`) and its Restore, Delete and Transfer buttons follow it. The roster in `/me/courses` and the
 * person's role decide, as they did before `can` (ADR-0004: the server still enforces it).
 */
export function useCourseAccess(courseId: string) {
  const course = useCourse(courseId);
  const me = useMe();
  const mine = useMyCourses();
  const can = course.data?.can ?? NOTHING;
  const user = me.data;
  const row = mine.data?.find((c) => c.id === courseId);
  const manageRoot = !!user && (user.role === "admin" || (!!row && (row.owner_id === user.id || user.role === "teacher")));
  const archived = course.data?.status === "archived";
  // Who else opens the settings of an archived course is known only once the person and their courses have loaded.
  const waiting = archived && !can.manage_course && (!me.data || !mine.data);
  return {
    can,
    staff: Object.values(can).some(Boolean),
    manageRoot,
    settings: can.manage_course || (archived && manageRoot),
    loading: (!course.data && !course.error) || waiting,
  };
}
