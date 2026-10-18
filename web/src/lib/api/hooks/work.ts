"use client";

import { api, type Page } from "../client";
import { localToIso } from "../../format";
import { useQuery } from "../../query";
import type { MyWork, SearchHit, TeachingOverview } from "../types";
import type { MyGrades } from "../types";

/** One call for every assignment of every course with the caller's status (ADR-0052). */
export function useMyWork() {
  // The server sends the class day as a date; the screens compare instants, so it becomes noon of that day.
  return useQuery<MyWork>("/me/work", async () => {
    const work = await api.get<MyWork>("/me/work");
    return { ...work, items: work.items.map((i) => ({ ...i, class_on: i.class_on ? localToIso(`${i.class_on}T12:00`) : null })) };
  });
}

/** Published grades only; a student never receives draft fields (root AGENTS.md section 6). */
export function useMyGrades(courseId: string | null) {
  return useQuery<MyGrades>(courseId ? `/courses/${courseId}/my-grades` : null, () => api.get<MyGrades>(`/courses/${courseId}/my-grades`));
}

/** The cross-course grading queue and the teacher home feed (ADR-0052). */
export function useTeachingOverview() {
  return useQuery<TeachingOverview>("/me/teaching", () => api.get<TeachingOverview>("/me/teaching"));
}

/** Search over the caller's courses and materials (ADR-0052). */
export function useSearch(q: string) {
  const term = q.trim();
  return useQuery<Page<SearchHit>>(term.length >= 2 ? `/search?q=${encodeURIComponent(term)}` : null, () => api.get<Page<SearchHit>>(`/search?q=${encodeURIComponent(term)}`));
}
