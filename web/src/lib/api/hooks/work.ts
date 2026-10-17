"use client";

import { api } from "../client";
import { useQuery } from "../../query";
import type { MyWork, TeachingOverview } from "../gaps";
import type { MyGrades } from "../types";

/** GAP: one call for every assignment of every course with the caller's status (API_GAPS.md, "My work"). */
export function useMyWork() {
  return useQuery<MyWork>("/me/work", () => api.get<MyWork>("/me/work"));
}

/** Published grades only; a student never receives draft fields (root AGENTS.md section 6). */
export function useMyGrades(courseId: string | null) {
  return useQuery<MyGrades>(courseId ? `/courses/${courseId}/my-grades` : null, () => api.get<MyGrades>(`/courses/${courseId}/my-grades`));
}

/** GAP: the cross-course grading queue and teacher home feed (API_GAPS.md, "Teaching overview"). */
export function useTeachingOverview() {
  return useQuery<TeachingOverview>("/me/teaching", () => api.get<TeachingOverview>("/me/teaching"));
}
