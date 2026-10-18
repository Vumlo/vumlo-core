"use client";

import { api } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import { configToWire, itemFromWire } from "../sheetWire";
import type { Document, EditorProfiles, Item, ItemRevision, Module } from "../types";

export type NewItem =
  | { kind: "text"; title: string; body?: Document }
  | { kind: "assignment"; title: string; body?: Document; assignment: { type: string; max_points: string; config?: Record<string, unknown>; due_at?: string | null; on_deadline?: string | null; accept_late?: boolean | null; max_attempts?: number | null } };

export type ItemPatch = Partial<{
  title: string;
  body: Document;
  config: Record<string, unknown>;
  /** The type whose screens shape `config` (`quiz` or `form`); it is converted to the server's shape (ADR-0050, ADR-0051). */
  configType: string;
  max_points: string;
  due_at: string | null;
  on_deadline: string | null;
  accept_late: boolean | null;
  late_until: string | null;
  max_attempts: number | null;
  publish_at: string | null;
  /** The day of in-class work, `YYYY-MM-DD`; `null` clears it (ADR-0049). */
  class_on: string | null;
}>;

const touched = (courseId: string) => {
  invalidate(`/courses/${courseId}`);
  invalidate("/me/");
  invalidate("/items/");
};

export function useEditorProfiles() {
  return useQuery<EditorProfiles>("/editor/profiles", () => api.get<EditorProfiles>("/editor/profiles"));
}

/** Whether a published item has changes that students do not see yet (ADR-0049). */
export function useItemRevision(itemId: string | null) {
  return useQuery<ItemRevision>(itemId ? `/items/${itemId}/revision` : null, () => api.get<ItemRevision>(`/items/${itemId}/revision`));
}

export function usePublishRevision(itemId: string, courseId: string) {
  return useMutation(async () => {
    const r = await api.post<ItemRevision>(`/items/${itemId}/revision/publish`);
    touched(courseId);
    return r;
  });
}

export function useCreateModule(courseId: string) {
  return useMutation(async (title: string, parentId?: string | null) => {
    const m = await api.post<Module>(`/courses/${courseId}/modules`, { title, parent_id: parentId ?? null });
    touched(courseId);
    return m;
  });
}

export function useUpdateModule(courseId: string) {
  return useMutation(async (moduleId: string, patch: { title?: string; publish_at?: string | null }) => {
    const m = await api.patch<Module>(`/modules/${moduleId}`, patch);
    touched(courseId);
    return m;
  });
}

export function useDeleteModule(courseId: string) {
  return useMutation(async (moduleId: string) => {
    await api.delete(`/modules/${moduleId}`);
    touched(courseId);
    return true as const;
  });
}

/** `cascade` has no default (ADR-0010): the dialog makes the teacher choose. */
export function usePublishModule(courseId: string) {
  return useMutation(async (moduleId: string, cascade: boolean) => {
    const m = await api.post<Module>(`/modules/${moduleId}/publish`, { cascade });
    touched(courseId);
    return m;
  });
}

export function useCreateItem(courseId: string) {
  return useMutation(async (moduleId: string, item: NewItem) => {
    const wire = item.kind === "assignment" && item.assignment.config ? { ...item, assignment: { ...item.assignment, config: configToWire(item.assignment.type, item.assignment.config) } } : item;
    const i = await api.post<Item>(`/modules/${moduleId}/items`, wire);
    touched(courseId);
    return itemFromWire(i);
  });
}

export function useUpdateItem(courseId: string) {
  return useMutation(async (itemId: string, patch: ItemPatch) => {
    const { configType, ...rest } = patch;
    const wire = rest.config && configType ? { ...rest, config: configToWire(configType, rest.config) } : rest;
    const i = await api.patch<Item>(`/items/${itemId}`, wire);
    touched(courseId);
    return itemFromWire(i);
  });
}

export function useDeleteItem(courseId: string) {
  return useMutation(async (itemId: string) => {
    await api.delete(`/items/${itemId}`);
    touched(courseId);
    return true as const;
  });
}

export function usePublishItem(courseId: string) {
  return useMutation(async (itemId: string) => {
    const i = await api.post<Item>(`/items/${itemId}/publish`);
    touched(courseId);
    return i;
  });
}

export function useReorderModules(courseId: string) {
  return useMutation(async (ids: string[], parentId?: string | null) => {
    await api.put(`/courses/${courseId}/modules/order`, { ids, parent_id: parentId ?? null });
    touched(courseId);
  });
}

export function useReorderItems(courseId: string) {
  return useMutation(async (moduleId: string, ids: string[]) => {
    await api.put(`/modules/${moduleId}/items/order`, { ids });
    touched(courseId);
  });
}
