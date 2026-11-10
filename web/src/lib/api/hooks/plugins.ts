"use client";

import { api, isDesktop } from "../client";
import { useQuery } from "../../query";
import type { Launch, PluginReports, PluginSlot, PluginSlots } from "../types";

/** The slot of a tab on the page of an assignment (ADR-0066). */
const ASSIGNMENT_TAB = "assignment.tab";

/**
 * The pages that enabled plugins offer in the tab of an assignment. The desktop app shows none and
 * does not ask: its CSP is fixed when it is built and its webview never touches the network.
 */
export function usePluginTabs(): PluginSlot[] {
  const path = `/plugin-slots?slot=${ASSIGNMENT_TAB}`;
  const slots = useQuery<PluginSlots>(isDesktop() ? null : path, () => api.get<PluginSlots>(path));
  return slots.data?.items ?? [];
}

/** A token, good for a minute, that tells the plugin's own server who looks at this assignment. */
export function requestLaunch(assignmentId: string, pluginId: string) {
  return api.post<Launch>(`/assignments/${assignmentId}/plugin-launch`, { plugin_id: pluginId });
}

/** What a report is about: the parts of the API that list `plugin-reports` (ADR-0077). */
export type ReportSubject = "submissions" | "assignments" | "courses";

/**
 * What the plugins reported about a subject, for staff who may see it. A null `id` asks nothing: a screen passes it
 * for anyone but staff, so a student's browser never makes the request.
 */
export function usePluginReports(subject: ReportSubject, id: string | null) {
  const path = `/${subject}/${id}/plugin-reports`;
  return useQuery<PluginReports>(id ? path : null, () => api.get<PluginReports>(path));
}
