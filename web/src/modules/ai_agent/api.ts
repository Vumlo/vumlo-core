"use client";

import { api } from "@/lib/api/client";
import { invalidate, useQuery } from "@/lib/query";

// The routes of the module `ai_agent` (ADR-0070). An optional module is not in the core's generated
// schema, so its few types are written here.
export type BuddyMessage = { id: string; role: "user" | "assistant"; content: string; created_at: string };
/** A change the Buddy prepared; it happens only when the person confirms. `facts` is what the card shows. */
export type Proposal = { id: string; tool: string; facts: Record<string, string>; created_at: string };
export type BuddyChat = { notice: string | null; messages: BuddyMessage[]; proposals: Proposal[] };
export type Exchange = { user: BuddyMessage; assistant: BuddyMessage; proposals: Proposal[] };
/** Where the person is. Hints only: every lookup is checked again for them on the server. */
export type PageContext = { page?: string; course_id?: string; item_id?: string };

export const CHAT = "/modules/ai_agent/chat";

/** The person's own chat. An error (404: the Buddy is off for their role) means there is nothing to show. */
export function useBuddyChat() {
  return useQuery<BuddyChat>(CHAT, () => api.get<BuddyChat>(CHAT));
}

export const sendBuddyMessage = (text: string, context: PageContext) => api.post<Exchange>("/modules/ai_agent/messages", { text, context });

/** What a confirm answers. `temporary_password` is set only by a password reset, and only this once. */
export type Confirmed = { temporary_password: string | null };
export const confirmProposal = (id: string) => api.post<Confirmed>(`/modules/ai_agent/proposals/${id}/confirm`);
export const rejectProposal = (id: string) => api.post<void>(`/modules/ai_agent/proposals/${id}/reject`);

export async function clearBuddyChat() {
  await api.delete(CHAT);
  invalidate(CHAT);
}

const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** The page that is open now, read when a message is sent (ids are in the query string, ADR-0057). */
export function pageContext(): PageContext {
  const { pathname, search } = window.location;
  const q = new URLSearchParams(search);
  const id = (v: string | null) => (v && ID.test(v) ? v : undefined);
  const onCourse = pathname === "/course" || pathname.startsWith("/course/");
  return { page: pathname, course_id: onCourse ? id(q.get("id")) : undefined, item_id: onCourse ? id(q.get("item")) : undefined };
}
