
import { api } from "@/lib/api/client";
import { invalidate, useMutation, useQuery } from "@/lib/query";
import type { AdminPlugin, AdminPluginCalls, AdminPluginSetting, PluginStatus } from "@/lib/api/types";

const base = (id: string) => `/admin/plugins/${encodeURIComponent(id)}`;

/** The installed plugins, for the console (ADR-0095). */
export function useAdminPlugins() {
  return useQuery<{ items: AdminPlugin[] }>("/admin/plugins", () => api.get<{ items: AdminPlugin[] }>("/admin/plugins"));
}

export function useAdminPlugin(id: string) {
  return useQuery<AdminPlugin>(base(id), () => api.get<AdminPlugin>(base(id)));
}

/** The institution settings a plugin's manifest declares. A secret carries no value, only whether it is set. */
export function useAdminPluginSettings(id: string) {
  const path = `${base(id)}/settings`;
  return useQuery<{ items: AdminPluginSetting[] }>(path, () => api.get<{ items: AdminPluginSetting[] }>(path));
}

export function useAdminPluginCalls(id: string) {
  const path = `${base(id)}/calls?limit=20`;
  return useQuery<AdminPluginCalls>(path, () => api.get<AdminPluginCalls>(path));
}

export function useSetPluginStatus(id: string) {
  return useMutation(async (status: PluginStatus) => {
    const plugin = await api.put<AdminPlugin>(`${base(id)}/status`, { status });
    invalidate("/admin/plugins");
    return plugin;
  });
}

export function useGrantPlugin(id: string) {
  return useMutation(async (scopes: string[], allCourses: boolean, courseIds: string[]) => {
    const plugin = await api.put<AdminPlugin>(`${base(id)}/grants`, { scopes, all_courses: allCourses, course_ids: courseIds });
    invalidate("/admin/plugins");
    return plugin;
  });
}

/** `value` is the text of the value: a string stays text, a number, a boolean or a list is JSON. */
export function useSetPluginSetting(id: string, key: string) {
  return useMutation(async (value: string) => {
    await api.put(`${base(id)}/settings/${encodeURIComponent(key)}`, { value });
    invalidate(`${base(id)}/settings`);
    invalidate("/admin/plugins");
    return true as const;
  });
}

export function useUnsetPluginSetting(id: string, key: string) {
  return useMutation(async () => {
    await api.delete(`${base(id)}/settings/${encodeURIComponent(key)}`);
    invalidate(`${base(id)}/settings`);
    invalidate("/admin/plugins");
    return true as const;
  });
}
