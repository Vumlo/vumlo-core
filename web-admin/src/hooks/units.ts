import { api } from "@/lib/api/client";
import { fetchAll } from "@/lib/api/hooks/util";
import type { components } from "@/lib/api/schema";
import { invalidate, useMutation, useQuery } from "@/lib/query";

type S = components["schemas"];
export type UnitSummary = S["UnitSummary"];
export type UnitRole = S["UnitRole"];
export type UnitAbility = S["UnitAbility"];
export type UnitRoleEntry = S["UnitRoleEntry"];
export type UnitRoleAbilities = S["UnitRoleAbilities"];
export type Candidate = S["Candidate"];

/** Every unit, flat and by name; the tree is built from `parent_id` (ADR-0094). */
export function useUnits() {
  return useQuery<UnitSummary[]>("/units", () => fetchAll<UnitSummary>("/units"));
}

export function useCreateUnit() {
  return useMutation(async (name: string, parentId: string | null) => {
    const unit = await api.post<S["Unit"]>("/units", { name, parent_id: parentId });
    invalidate("/units");
    return unit;
  });
}

/** `parentId` undefined leaves the unit where it is; `null` moves it to the top. */
export function useUpdateUnit(id: string) {
  return useMutation(async (patch: { name?: string; parentId?: string | null }) => {
    const unit = await api.patch<S["Unit"]>(`/units/${id}`, { ...(patch.name !== undefined ? { name: patch.name } : {}), ...(patch.parentId !== undefined ? { parent_id: patch.parentId } : {}) });
    invalidate("/units");
    return unit;
  });
}

export function useDeleteUnit(id: string) {
  return useMutation(async () => {
    await api.delete(`/units/${id}`);
    invalidate("/units");
    return true as const;
  });
}

export function useUnitRoles(id: string | null) {
  const path = `/units/${id}/roles`;
  return useQuery<UnitRoleEntry[]>(id ? path : null, () => fetchAll<UnitRoleEntry>(path));
}

export function useSetUnitRole(id: string) {
  return useMutation(async (userId: string, role: UnitRole) => {
    await api.put(`/units/${id}/roles/${userId}`, { role });
    invalidate(`/units/${id}/`);
    return true as const;
  });
}

export function useRemoveUnitRole(id: string) {
  return useMutation(async (userId: string) => {
    await api.delete(`/units/${id}/roles/${userId}`);
    invalidate(`/units/${id}/`);
    return true as const;
  });
}

/** Teachers and administrators without a role on the unit who match `q`, at most 20. */
export function useUnitCandidates(id: string, q: string) {
  const path = `/units/${id}/candidates?q=${encodeURIComponent(q)}`;
  return useQuery<{ items: Candidate[] }>(q.trim() ? path : null, () => api.get<{ items: Candidate[] }>(path));
}

export function useRoleAbilities() {
  return useQuery<UnitRoleAbilities>("/admin/unit-roles", () => api.get<UnitRoleAbilities>("/admin/unit-roles"));
}

/** Replaces the switches of a role, for every unit (ADR-0094). */
export function useSetRoleAbilities(role: UnitRole) {
  return useMutation(async (abilities: UnitAbility[]) => {
    const all = await api.put<UnitRoleAbilities>(`/admin/unit-roles/${role}`, { abilities });
    invalidate("/admin/unit-roles");
    return all;
  });
}

