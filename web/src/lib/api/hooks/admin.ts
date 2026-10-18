"use client";

import { api } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import type {
  AdminCourse,
  AdminInstitution,
  AuditKind,
  AuditRecord,
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

/** Every course of the institution (ADR-0053). */
export function useAdminCourses() {
  return useQuery<AdminCourse[]>("/admin/courses", () => fetchAll<AdminCourse>("/admin/courses"));
}

export function useCreateTerm() {
  return useMutation(async (body: { name: string; starts_on: string; ends_on: string }) => {
    const t = await api.post<Term>("/terms", body);
    invalidate("/terms");
    invalidate("/admin/");
    return t;
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

export function useChangeOwner() {
  return useMutation(async (courseId: string, userId: string) => {
    const c = await api.put<unknown>(`/courses/${courseId}/owner`, { user_id: userId });
    invalidate("/admin/");
    invalidate("/courses/");
    invalidate("/me/");
    return c;
  });
}
