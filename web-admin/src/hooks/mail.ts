import { api } from "@/lib/api/client";
import type { components } from "@/lib/api/schema";
import { invalidate, useMutation, useQuery } from "@/lib/query";

type S = components["schemas"];
export type MailStatus = S["MailStatus"];
export type MailMessage = S["MailMessage"];

export type MailFilter = { state: string; plugin: string };

/** What waits, what was sent and what died in the last day, and whether mail is set up (ADR-0095). */
export function useMailStatus() {
  return useQuery<MailStatus>("/admin/mail/status", () => api.get<MailStatus>("/admin/mail/status"));
}

/** The newest messages of the journal, by state and plugin. A person is an id, never an address. */
export function useMailMessages(filter: MailFilter) {
  const q = new URLSearchParams({ limit: "50" });
  if (filter.state) q.set("state", filter.state);
  if (filter.plugin.trim()) q.set("plugin_id", filter.plugin.trim());
  const path = `/admin/mail/messages?${q}`;
  return useQuery<{ items: MailMessage[] }>(path, () => api.get<{ items: MailMessage[] }>(path));
}

/** Puts every dead message back in the queue; the answer says how many, and how many could not be. */
export function useRetryMail() {
  return useMutation(async () => {
    const answer = await api.post<{ requeued: number; erased: number }>("/admin/mail/retry", {});
    invalidate("/admin/mail");
    return answer;
  });
}
