"use client";

import { api } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import { localToIso } from "../../format";

import type { Candidate, ContentTree, Course, CourseCard, CourseGroupLink, CourseInfo, CoursePeople, GroupOption, Item, ItemMetaMap, Membership, ServerItemMeta, TemplateSummary, Term } from "../types";
import { itemFromWire } from "../sheetWire";
import { fetchAll } from "./util";

export function useMyCourses() {
  return useQuery<Course[]>("/me/courses", () => fetchAll<Course>("/me/courses"));
}

export function useCourse(id: string | null) {
  return useQuery<Course>(id ? `/courses/${id}` : null, () => api.get<Course>(`/courses/${id}`));
}

/** The header of every course screen: code, term name, owner and counts (ADR-0048). */
export function useCourseInfo(id: string | null) {
  return useQuery<CourseInfo>(id ? `/courses/${id}/info` : null, () => api.get<CourseInfo>(`/courses/${id}/info`));
}

/** The course cards: progress for a student, a to-do list for staff (ADR-0052). */
export function useCourseCards() {
  return useQuery<CourseCard[]>("/me/course-cards", async () => (await api.get<{ items: CourseCard[] }>("/me/course-cards")).items);
}

/** The caller's templates with their content counts (ADR-0048). */
export function useTemplates() {
  return useQuery<TemplateSummary[]>("/me/templates", () => fetchAll<TemplateSummary>("/me/templates"));
}

export function useContentTree(courseId: string | null) {
  return useQuery<ContentTree>(courseId ? `/courses/${courseId}/content` : null, () => api.get<ContentTree>(`/courses/${courseId}/content`));
}

/**
 * Read time, class day, student progress and staff counts per item (ADR-0049). The server sends the
 * class day as a date; the screens get it as noon of that day in the institution's zone.
 */
export function useItemMeta(courseId: string | null) {
  return useQuery<ItemMetaMap>(courseId ? `/courses/${courseId}/item-meta` : null, async () => {
    const wire = await api.get<{ items: Record<string, ServerItemMeta> }>(`/courses/${courseId}/item-meta`);
    return {
      items: Object.fromEntries(
        Object.entries(wire.items).map(([id, meta]) => [id, { ...meta, class_on: meta.class_on ? localToIso(`${meta.class_on}T12:00`) : null }]),
      ),
    };
  });
}

export function useItem(id: string | null) {
  return useQuery<Item>(id ? `/items/${id}` : null, async () => itemFromWire(await api.get<Item>(`/items/${id}`)));
}

/** The roster with names; staff also get emails, former students and linked groups (ADR-0048). */
export function useCoursePeople(courseId: string | null) {
  return useQuery<CoursePeople>(courseId ? `/courses/${courseId}/people` : null, () => api.get<CoursePeople>(`/courses/${courseId}/people`));
}

export function useMembers(courseId: string | null) {
  return useQuery<Membership[]>(courseId ? `/courses/${courseId}/members` : null, () => fetchAll<Membership>(`/courses/${courseId}/members`));
}

export function useGroupOptions(courseId: string | null) {
  return useQuery<GroupOption[]>(courseId ? `/courses/${courseId}/group-options` : null, () => fetchAll<GroupOption>(`/courses/${courseId}/group-options`));
}

export function useTerms() {
  return useQuery<Term[]>("/terms", () => fetchAll<Term>("/terms"));
}

const changed = (courseId: string) => {
  invalidate(`/courses/${courseId}`);
  invalidate("/me/");
};

export function useUpdateCourse(courseId: string) {
  return useMutation(async (patch: { title?: string; term_id?: string | null }) => {
    const c = await api.patch<Course>(`/courses/${courseId}`, patch);
    changed(courseId);
    return c;
  });
}

export function useUpdateCourseInfo(courseId: string) {
  return useMutation(async (patch: { code?: string; description?: string }) => {
    const r = await api.patch<CourseInfo>(`/courses/${courseId}/info`, patch);
    changed(courseId);
    return r;
  });
}

export function useCreateCourse() {
  return useMutation(async (title: string, term_id?: string | null) => {
    const c = await api.post<Course>("/courses", { title, term_id: term_id ?? null });
    invalidate("/me/");
    return c;
  });
}

export function useCreateFromTemplate() {
  return useMutation(async (templateId: string, title: string) => {
    const c = await api.post<Course>(`/templates/${templateId}/courses`, { title });
    invalidate("/me/");
    return c;
  });
}

export function useMakeTemplate(courseId: string) {
  return useMutation(async () => {
    const c = await api.post<Course>(`/courses/${courseId}/template`);
    invalidate("/me/");
    return c;
  });
}

/** An archived course stays readable and takes no changes; restoring publishes it (ADR-0048). */
export function useArchiveCourse(courseId: string) {
  return useMutation(async (archive: boolean) => {
    const c = await api.post<Course>(`/courses/${courseId}/${archive ? "archive" : "restore"}`);
    changed(courseId);
    return c;
  });
}

export function usePublishCourse(courseId: string) {
  return useMutation(async () => {
    const c = await api.post<Course>(`/courses/${courseId}/publish`);
    changed(courseId);
    return c;
  });
}

export function useTransferOwner(courseId: string) {
  return useMutation(async (userId: string) => {
    const c = await api.put<Course>(`/courses/${courseId}/owner`, { user_id: userId });
    changed(courseId);
    return c;
  });
}

export function usePutMember(courseId: string) {
  return useMutation(async (userId: string, role: "teacher" | "assistant" | "student") => {
    const m = await api.put<Membership>(`/courses/${courseId}/members/${userId}`, { role });
    changed(courseId);
    return m;
  });
}

export function useRemoveMember(courseId: string) {
  return useMutation(async (userId: string) => {
    await api.delete(`/courses/${courseId}/members/${userId}`);
    changed(courseId);
  });
}

export function useLinkGroup(courseId: string) {
  return useMutation(async (groupId: string, subgroupId?: string | null) => {
    const l = await api.post<CourseGroupLink>(`/courses/${courseId}/groups`, { group_id: groupId, subgroup_id: subgroupId ?? null });
    changed(courseId);
    return l;
  });
}

export function useUnlinkGroup(courseId: string) {
  return useMutation(async (linkId: string) => {
    await api.delete(`/courses/${courseId}/groups/${linkId}`);
    changed(courseId);
  });
}

/** People to add to the course: at least two characters, at most 20 answers (ADR-0048). */
export function useCandidates(courseId: string | null, q: string) {
  return useQuery<Candidate[]>(courseId ? `/courses/${courseId}/candidates?q=${encodeURIComponent(q)}` : null, async () => (await api.get<{ items: Candidate[] }>(`/courses/${courseId}/candidates?q=${encodeURIComponent(q)}`)).items);
}

/** `409` once a grade has a value or work was submitted: archive the course then. */
export function useDeleteCourse(courseId: string) {
  return useMutation(async () => {
    await api.delete(`/courses/${courseId}`);
    invalidate("/me/");
    return true as const;
  });
}
