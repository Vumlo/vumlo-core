"use client";

import { api } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import type { GradebookGroupInput, GradebookSettings, GradebookStructure } from "../types";

/** Groups, rules and what is left to sort, for the Structure mode of the gradebook (ADR-0089). Staff only. */
export function useGradebookStructure(courseId: string | null, enabled = true) {
  const path = `/courses/${courseId}/gradebook/structure`;
  return useQuery<GradebookStructure>(courseId && enabled ? path : null, () => api.get<GradebookStructure>(path));
}

/** A change to how marks add up changes every total, so every cached read of the course and of "me" is stale. */
function touched() {
  invalidate("/courses/");
  invalidate("/me/");
}

export function useSaveGradebookSettings(courseId: string) {
  return useMutation(async (settings: GradebookSettings) => {
    const r = await api.put<GradebookSettings>(`/courses/${courseId}/gradebook/settings`, settings);
    touched();
    return r;
  });
}

export function useCreateGradebookGroup(courseId: string) {
  return useMutation(async (input: GradebookGroupInput) => {
    const r = await api.post<{ key: string }>(`/courses/${courseId}/gradebook/groups`, input);
    touched();
    return r;
  });
}

/** Replaces the rules of a group: a module's group in the `course` layout, the teacher's own in `custom`. */
export function useSaveGradebookGroup(courseId: string) {
  return useMutation(async (key: string, input: GradebookGroupInput) => {
    const r = await api.put<unknown>(`/courses/${courseId}/gradebook/groups/${key}`, input);
    touched();
    return r;
  });
}

export function useDeleteGradebookGroup(courseId: string) {
  return useMutation(async (key: string) => {
    await api.delete(`/courses/${courseId}/gradebook/groups/${key}`);
    touched();
    return true as const;
  });
}

/** Puts assignments in a group of the teacher's own; `group: null` sends them back to "Not sorted". */
export function usePlaceAssignments(courseId: string) {
  return useMutation(async (assignments: string[], group: string | null) => {
    await api.put(`/courses/${courseId}/gradebook/placement`, { assignments, group });
    touched();
    return true as const;
  });
}
