"use client";

import { api, isDesktop } from "../client";
import { useQuery } from "../../query";
import type { Launch, PluginSlot, PluginSlots } from "../types";

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
