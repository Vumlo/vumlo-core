"use client";

import { api } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import type {
  AdminCourseRow,
  AdminGroupOverview,
  AdminTermStat,
  AdminUserFacts,
  AdminUserSummary,
  AuditEvent,
  AuditKind,
  InstitutionAdmin,
} from "../gaps";
import type { CreateUserResponse, Group, Role, Subgroup, Term, User } from "../types";
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

/** GAP: counts per role for the filter chips (API_GAPS.md, "Admin people"). */
export function useAdminUserSummary() {
  return useQuery<AdminUserSummary>("/admin/users/summary", () => api.get<AdminUserSummary>("/admin/users/summary"));
}

/** GAP: last sign-in, groups and courses of a person (API_GAPS.md, "Admin people"). */
export function useAdminUserFacts(id: string | null) {
  return useQuery<AdminUserFacts>(id ? `/admin/users/${id}/facts` : null, () => api.get<AdminUserFacts>(`/admin/users/${id}/facts`));
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

/** GAP: members with emails and join dates, linked courses (API_GAPS.md, "Admin groups"). */
export function useGroupOverview(groupId: string | null) {
  return useQuery<AdminGroupOverview>(groupId ? `/admin/groups/${groupId}/overview` : null, () => api.get<AdminGroupOverview>(`/admin/groups/${groupId}/overview`));
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

/** GAP: course counts per term (API_GAPS.md, "Admin terms and courses"). */
export function useAdminTermStats() {
  return useQuery<AdminTermStat[]>("/admin/term-stats", async () => (await api.get<{ items: AdminTermStat[] }>("/admin/term-stats")).items);
}

/** GAP: every course of the institution (API_GAPS.md, "Admin terms and courses"). */
export function useAdminCourses() {
  return useQuery<AdminCourseRow[]>("/admin/courses", async () => (await api.get<{ items: AdminCourseRow[] }>("/admin/courses")).items);
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
  return useQuery<InstitutionAdmin>("/admin/institution", () => api.get<InstitutionAdmin>("/admin/institution"));
}

/** GAP: institution settings are config-file only today (API_GAPS.md, "Institution"). */
export function useSaveInstitution() {
  return useMutation(async (patch: Partial<Pick<InstitutionAdmin, "name" | "short_name" | "accent" | "language" | "time_zone">>) => {
    const r = await api.patch<InstitutionAdmin>("/admin/institution", patch);
    invalidate("/admin/institution");
    invalidate("/institution");
    return r;
  });
}

/** GAP: the audit log has no read endpoint (API_GAPS.md, "Audit log"). */
export function useAuditLog(filter: { q: string; kind: AuditKind | "all"; days: number }) {
  const key = `/admin/audit?q=${encodeURIComponent(filter.q)}&kind=${filter.kind}&days=${filter.days}`;
  return useQuery<AuditEvent[]>(key, async () => (await api.get<{ items: AuditEvent[] }>(key)).items);
}

/** Student and course counts of several groups at once, for the group list (API_GAPS.md, "Admin groups"). */
export function useGroupOverviews(ids: string[]) {
  return useQuery<Record<string, AdminGroupOverview>>(ids.length ? `/admin/groups/overviews?ids=${ids.join(",")}` : null, async () => {
    const entries = await Promise.all(ids.map(async (id) => [id, await api.get<AdminGroupOverview>(`/admin/groups/${id}/overview`)] as const));
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
