import { http, HttpResponse } from "msw";
import { getDb } from "@/mocks/db";
import { problem, requireAdmin } from "@/mocks/logic";

const A = "/api/v1/admin/mail";
type State = "pending" | "sent" | "dead" | "cancelled";
type Message = { id: string; created_at: string; state: State; attempts: number; plugin_id: string | null; kind: string; subject: string | null; last_error: string | null; erased: boolean };

// The queue of the mock: one waiting, two sent and three dead (two of `ocr`, one of the core, whose address is
// already erased and so cannot be put back). `localStorage["vumlo.mock.mail"] = "off"` is an installation without [mail].
const messages: Message[] = [
  { id: "m1", created_at: "2026-11-13T09:00:00Z", state: "pending", attempts: 0, plugin_id: "ocr", kind: "grade", subject: "Your grade is ready", last_error: null, erased: false },
  { id: "m2", created_at: "2026-11-13T08:50:00Z", state: "sent", attempts: 1, plugin_id: "ocr", kind: "grade", subject: "Your grade is ready", last_error: null, erased: false },
  { id: "m3", created_at: "2026-11-13T08:40:00Z", state: "sent", attempts: 1, plugin_id: null, kind: "account", subject: "Verify your address", last_error: null, erased: false },
  { id: "m4", created_at: "2026-11-13T08:30:00Z", state: "dead", attempts: 5, plugin_id: "ocr", kind: "grade", subject: "Your grade is ready", last_error: "smtp 451", erased: false },
  { id: "m5", created_at: "2026-11-13T08:20:00Z", state: "dead", attempts: 5, plugin_id: "ocr", kind: "deadline", subject: "A deadline moved", last_error: "connect", erased: false },
  { id: "m6", created_at: "2026-11-13T08:10:00Z", state: "dead", attempts: 5, plugin_id: null, kind: "account", subject: null, last_error: "smtp 550", erased: true },
];

const configured = () => localStorage.getItem("vumlo.mock.mail") !== "off";
const count = (state: State) => messages.filter((m) => m.state === state).length;

export const mailHandlers = [
  http.get(`${A}/status`, () => {
    requireAdmin();
    const dead = messages.filter((m) => m.state === "dead");
    const by = new Map<string, { plugin_id: string | null; kind: string; dead: number }>();
    for (const m of dead) {
      const key = `${m.plugin_id}/${m.kind}`;
      by.set(key, { plugin_id: m.plugin_id, kind: m.kind, dead: (by.get(key)?.dead ?? 0) + 1 });
    }
    return HttpResponse.json({
      configured: configured(), transport: configured() ? "smtp" : null,
      pending: count("pending"), sent_24h: count("sent"), dead_24h: count("dead"), cancelled_24h: count("cancelled"), dead_now: count("dead"),
      oldest_pending: messages.find((m) => m.state === "pending")?.created_at ?? null, dead_by: [...by.values()],
    });
  }),
  http.get(`${A}/messages`, ({ request }) => {
    requireAdmin();
    const q = new URL(request.url).searchParams;
    const state = q.get("state");
    if (state && !["pending", "sent", "dead", "cancelled"].includes(state)) throw problem(422, "Validation failed", undefined, [{ field: "state", message: "must be pending, sent, dead or cancelled" }]);
    const teacher = getDb().users.find((u) => u.role === "teacher")?.id ?? "";
    const items = messages
      .filter((m) => (!state || m.state === state) && (!q.get("plugin_id") || m.plugin_id === q.get("plugin_id")) && (!q.get("kind") || m.kind === q.get("kind")))
      .map(({ erased: _erased, ...m }) => ({ ...m, user_id: teacher }));
    return HttpResponse.json({ items });
  }),
  http.post(`${A}/retry`, async ({ request }) => {
    requireAdmin();
    const b = (await request.json()) as { id?: string; plugin_id?: string };
    const scope = messages.filter((m) => m.state === "dead" && (b.id ? m.id === b.id : !b.plugin_id || m.plugin_id === b.plugin_id));
    const live = scope.filter((m) => !m.erased);
    for (const m of live) m.state = "pending";
    return HttpResponse.json({ requeued: live.length, erased: scope.length - live.length });
  }),
];
