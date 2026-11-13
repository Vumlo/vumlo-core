"use client";

import { useState } from "react";
import { api, type Page } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import type {
  AdminCourse,
  AdminInstitution,
  AppDownload,
  AuditKind,
  AuditRecord,
  CourseStatus,
  CreateUserResponse,
  Group,
  GroupOverview,
  Role,
  Subgroup,
  Term,
  TermStat,
  User,
  UserFacts,
  UserSummary,
} from "../types";
import { fetchAll } from "./util";

export type UserFilter = { role?: Role; status?: "disabled"; email?: string };

export function useAdminUsers(filter: UserFilter) {
  const q = new URLSearchParams();
  if (filter.role) q.set("role", filter.role);
  if (filter.status) q.set("status", filter.status);
  if (filter.email) q.set("email", filter.email);
  const qs = q.toString();
  const key = `/admin/users${qs ? `?${qs}` : ""}`;
  return useQuery<User[]>(key, () => fetchAll<User>(key));
}

/** People by role and the disabled, for the filter chips (ADR-0053). */
export function useAdminUserSummary() {
  return useQuery<UserSummary>("/admin/users/summary", () => api.get<UserSummary>("/admin/users/summary"));
}

/** Last sign-in, groups and courses of a person (ADR-0053). */
export function useAdminUserFacts(id: string | null) {
  return useQuery<UserFacts>(id ? `/admin/users/${id}/facts` : null, () => api.get<UserFacts>(`/admin/users/${id}/facts`));
}

export function useCreateUser() {
  return useMutation(async (body: { email: string; name: string; surname: string; role: Role }) => {
    const r = await api.post<CreateUserResponse>("/admin/users", body);
    invalidate("/admin/users");
    return r;
  });
}

export function useUpdateUser() {
  return useMutation(async (id: string, patch: { name?: string; surname?: string; role?: Role; status?: "active" | "disabled" }) => {
    const u = await api.patch<User>(`/admin/users/${id}`, patch);
    invalidate("/admin/");
    return u;
  });
}

export function useResetPassword() {
  return useMutation(async (id: string) => {
    const r = await api.post<{ temporary_password: string }>(`/admin/users/${id}/reset-password`);
    invalidate("/admin/");
    return r;
  });
}

export function useGroups() {
  return useQuery<Group[]>("/groups", () => fetchAll<Group>("/groups"));
}

export function useSubgroups(groupId: string | null) {
  return useQuery<Subgroup[]>(groupId ? `/groups/${groupId}/subgroups` : null, () => fetchAll<Subgroup>(`/groups/${groupId}/subgroups`));
}

/** Members with emails and join dates, and the linked courses (ADR-0053). */
export function useGroupOverview(groupId: string | null) {
  return useQuery<GroupOverview>(groupId ? `/admin/groups/${groupId}/overview` : null, () => api.get<GroupOverview>(`/admin/groups/${groupId}/overview`));
}

export function useCreateGroup() {
  return useMutation(async (name: string) => {
    const g = await api.post<Group>("/groups", { name });
    invalidate("/groups");
    invalidate("/admin/groups");
    return g;
  });
}

export function useAddGroupMembers(groupId: string) {
  return useMutation(async (userIds: string[]) => {
    const r = await api.post<{ added: number }>(`/groups/${groupId}/members`, { user_ids: userIds });
    invalidate("/groups");
    invalidate("/admin/groups");
    return r;
  });
}

export function useRemoveGroupMembers(groupId: string) {
  return useMutation(async (userIds: string[]) => {
    const r = await api.post<{ removed: number }>(`/groups/${groupId}/members/remove`, { user_ids: userIds });
    invalidate("/groups");
    invalidate("/admin/groups");
    return r;
  });
}

export function useReplaceGroupMembers(groupId: string) {
  return useMutation(async (userIds: string[]) => {
    const r = await api.put<{ added: number; removed: number }>(`/groups/${groupId}/members`, { user_ids: userIds });
    invalidate("/groups");
    invalidate("/admin/groups");
    return r;
  });
}

/** Course counts per term (ADR-0053). */
export function useAdminTermStats() {
  return useQuery<TermStat[]>("/admin/term-stats", async () => (await api.get<{ items: TermStat[] }>("/admin/term-stats")).items);
}

export type AllCoursesFilter = { termId: string; status: CourseStatus | "all"; q: string };

const ALL_COURSES_PAGE = 50;

/**
 * Every course of the institution, newest first, narrowed and paged by the server (ADR-0053): the
 * first page by its query, then `loadMore` appends the next ones. A change of the filter, or a
 * reload of the first page, starts again from the first page.
 */
export function useAllCourses(filter: AllCoursesFilter) {
  const params = new URLSearchParams();
  if (filter.termId) params.set("term_id", filter.termId);
  if (filter.status !== "all") params.set("status", filter.status);
  if (filter.q) params.set("q", filter.q);
  params.set("limit", String(ALL_COURSES_PAGE));
  const key = `/admin/courses?${params}`;
  const first = useQuery<Page<AdminCourse>>(key, () => api.get<Page<AdminCourse>>(key));
  const [more, setMore] = useState<{ key: string; base: Page<AdminCourse>; items: AdminCourse[]; cursor: string | null } | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [moreFailed, setMoreFailed] = useState(false);
  const extra = more && more.key === key && more.base === first.data ? more : null;
  const cursor = extra ? extra.cursor : (first.data?.next_cursor ?? null);
  const loadMore = async () => {
    if (!cursor || !first.data) return;
    setLoadingMore(true);
    setMoreFailed(false);
    try {
      const page = await api.get<Page<AdminCourse>>(`${key}&cursor=${encodeURIComponent(cursor)}`);
      setMore({ key, base: first.data, items: [...(extra?.items ?? []), ...page.items], cursor: page.next_cursor });
    } catch {
      setMoreFailed(true);
    } finally {
      setLoadingMore(false);
    }
  };
  return {
    data: first.data ? [...first.data.items, ...(extra?.items ?? [])] : undefined,
    error: first.error,
    hasMore: cursor !== null,
    loadMore,
    loadingMore,
    moreFailed,
  };
}

export function useCreateTerm() {
  return useMutation(async (body: { name: string; starts_on: string; ends_on: string }) => {
    const t = await api.post<Term>("/terms", body);
    invalidate("/terms");
    invalidate("/admin/");
    return t;
  });
}

/** Sets the whole list of links to the desktop app's installers (ADR-0064); a platform that is missing is removed. */
export function useSetAppDownloads() {
  return useMutation(async (downloads: AppDownload[]) => {
    const saved = await api.put<AppDownload[]>("/admin/institution/app-downloads", { downloads });
    invalidate("/institution");
    invalidate("/admin/institution");
    return saved;
  });
}

export function useAdminInstitution() {
  return useQuery<AdminInstitution>("/admin/institution", () => api.get<AdminInstitution>("/admin/institution"));
}

/** The audit log, newest first (ADR-0053). */
export function useAuditLog(filter: { q: string; kind: AuditKind | "all"; days: number }) {
  const key = `/admin/audit?q=${encodeURIComponent(filter.q)}&kind=${filter.kind}&days=${filter.days}`;
  return useQuery<AuditRecord[]>(key, () => fetchAll<AuditRecord>(key));
}

/** Student and course counts of several groups at once, for the group list (API_GAPS.md, "Admin groups"). */
export function useGroupOverviews(ids: string[]) {
  return useQuery<Record<string, GroupOverview>>(ids.length ? `/admin/groups/overviews?ids=${ids.join(",")}` : null, async () => {
    const entries = await Promise.all(ids.map(async (id) => [id, await api.get<GroupOverview>(`/admin/groups/${id}/overview`)] as const));
    return Object.fromEntries(entries);
  });
}
