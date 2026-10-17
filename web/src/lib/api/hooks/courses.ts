"use client";

import { api } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import type { Candidate, CourseCard, CourseInfo, CoursePeople, ItemMetaMap, Template } from "../gaps";
import type { ContentTree, Course, CourseGroupLink, GroupOption, Item, Membership, Term } from "../types";
import { fetchAll } from "./util";

export function useMyCourses() {
  return useQuery<Course[]>("/me/courses", () => fetchAll<Course>("/me/courses"));
}

export function useCourse(id: string | null) {
  return useQuery<Course>(id ? `/courses/${id}` : null, () => api.get<Course>(`/courses/${id}`));
}

/** GAP: code, term name, owner name and counts (API_GAPS.md, "Course info"). */
export function useCourseInfo(id: string | null) {
  return useQuery<CourseInfo>(id ? `/courses/${id}/info` : null, () => api.get<CourseInfo>(`/courses/${id}/info`));
}

/** GAP: aggregate for the course cards (API_GAPS.md, "Course cards"). */
export function useCourseCards() {
  return useQuery<CourseCard[]>("/me/course-cards", async () => (await api.get<{ items: CourseCard[] }>("/me/course-cards")).items);
}

/** GAP: templates with their content counts (API_GAPS.md, "Templates"). */
export function useTemplates() {
  return useQuery<Template[]>("/me/templates", async () => (await api.get<{ items: Template[] }>("/me/templates")).items);
}

export function useContentTree(courseId: string | null) {
  return useQuery<ContentTree>(courseId ? `/courses/${courseId}/content` : null, () => api.get<ContentTree>(`/courses/${courseId}/content`));
}

/** GAP: read time, class date, student progress and staff counts per item (API_GAPS.md, "Item meta"). */
export function useItemMeta(courseId: string | null) {
  return useQuery<ItemMetaMap>(courseId ? `/courses/${courseId}/item-meta` : null, () => api.get<ItemMetaMap>(`/courses/${courseId}/item-meta`));
}

export function useItem(id: string | null) {
  return useQuery<Item>(id ? `/items/${id}` : null, () => api.get<Item>(`/items/${id}`));
}

/** GAP: the members endpoint has ids only (API_GAPS.md, "People"). */
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

/** GAP: code and description are not on the course (API_GAPS.md, "Course info"). */
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

/** GAP: copy a template into a new course (API_GAPS.md, "Templates"). */
export function useCreateFromTemplate() {
  return useMutation(async (templateId: string, title: string) => {
    const c = await api.post<Course>(`/templates/${templateId}/courses`, { title });
    invalidate("/me/");
    return c;
  });
}

/** GAP: make a template from a course (API_GAPS.md, "Templates"). */
export function useMakeTemplate(courseId: string) {
  return useMutation(async () => {
    const c = await api.post<Course>(`/courses/${courseId}/template`);
    invalidate("/me/");
    return c;
  });
}

/** GAP: archive and restore are not in the API (API_GAPS.md, "Archive"). */
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

/** GAP: teachers cannot list users, so there is no way to find someone to add (API_GAPS.md, "People"). */
export function useCandidates(courseId: string | null, q: string) {
  return useQuery<Candidate[]>(courseId ? `/courses/${courseId}/candidates?q=${encodeURIComponent(q)}` : null, async () => (await api.get<{ items: Candidate[] }>(`/courses/${courseId}/candidates?q=${encodeURIComponent(q)}`)).items);
}

/** GAP: deleting a course has no endpoint (API_GAPS.md, "Archive and delete"). */
export function useDeleteCourse(courseId: string) {
  return useMutation(async () => {
    await api.delete(`/courses/${courseId}`);
    invalidate("/me/");
    return true as const;
  });
}
