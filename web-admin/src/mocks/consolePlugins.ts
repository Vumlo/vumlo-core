import { http, HttpResponse } from "msw";
import type { AdminPlugin, AdminPluginSetting } from "@/lib/api/types";
import { MOCK_NOW } from "@/lib/clock";
import { problem, requireAdmin } from "@/mocks/logic";

const A = "/api/v1/admin/plugins";

type Spec = Omit<AdminPluginSetting, "is_set" | "value">;
type Install = AdminPlugin & { specs: Spec[]; values: Record<string, string>; calls: { method: string; route: string; status: number; calls: number; average_ms: number }[] };

const spec = (key: string, kind: string, extra: Partial<Spec> = {}): Spec => ({
  key, description: "", kind, secret: false, options: null, minimum: null, maximum: null, item_kind: null, default: null, ...extra,
});

// Two installs, as the console sees them: `ocr`, short of a token and a limit, and `notify`, switched off. Settings are
// kept as the text they were typed as, which is how the server takes them (ADR-0072).
const installs: Install[] = [
  {
    plugin_id: "ocr", version: "1.2.0", base_url: "https://ocr.example.edu", status: "enabled",
    requested_scopes: ["grades:read", "settings:read", "submissions:read"], scopes: ["grades:read", "settings:read"],
    all_courses: true, course_ids: [], pending: 0, failed: 2, needs_setup: [],
    specs: [
      spec("api_token", "string", { secret: true, description: "The token of the OCR service" }),
      spec("mode", "string", { options: ["fast", "accurate"], default: "fast", description: "How hard it works on a scan" }),
      spec("limit", "number", { minimum: 1, maximum: 10, description: "Scans at once" }),
      spec("verbose", "boolean", { default: false, description: "Write every step to the log" }),
      spec("domains", "array", { item_kind: "string", description: "Hosts it may fetch files from, one per line" }),
    ],
    values: {},
    calls: [
      { method: "GET", route: "/plugin-api/settings", status: 200, calls: 12, average_ms: 4 },
      { method: "GET", route: "/plugin-api/grades", status: 403, calls: 2, average_ms: 3 },
    ],
  },
  {
    plugin_id: "notify", version: "0.3.1", base_url: "https://notify.example.edu", status: "disabled",
    requested_scopes: ["mail:send"], scopes: ["mail:send"], all_courses: false, course_ids: [], pending: 0, failed: 0, needs_setup: [],
    specs: [], values: {}, calls: [],
  },
];

const find = (id: string) => installs.find((i) => i.plugin_id === id) ?? (() => { throw problem(404, "Not found"); })();

const isSet = (i: Install, s: Spec) => s.key in i.values;
const needs = (i: Install) => i.specs.filter((s) => !isSet(i, s) && s.default === null).map((s) => s.key);
const view = (i: Install): AdminPlugin => ({
  plugin_id: i.plugin_id, version: i.version, base_url: i.base_url, status: i.status, requested_scopes: i.requested_scopes,
  scopes: i.scopes, all_courses: i.all_courses, course_ids: i.course_ids, pending: i.pending, failed: i.failed, needs_setup: needs(i),
});

function parse(s: Spec, text: string): unknown {
  const bad = (message: string) => problem(422, "Validation failed", undefined, [{ field: "value", message }]);
  if (s.kind === "number") {
    const n = Number(text);
    if (text.trim() === "" || Number.isNaN(n)) throw bad(`the value of \`${s.key}\` must be a number`);
    if ((s.minimum != null && n < s.minimum) || (s.maximum != null && n > s.maximum)) throw bad(`the value of \`${s.key}\` must be between ${s.minimum} and ${s.maximum}`);
    return n;
  }
  if (s.kind === "boolean") {
    if (text !== "true" && text !== "false") throw bad(`the value of \`${s.key}\` must be true or false`);
    return text === "true";
  }
  if (s.kind === "array") {
    try {
      const list: unknown = JSON.parse(text);
      if (Array.isArray(list)) return list;
    } catch {
      // fall through to the refusal below
    }
    throw bad(`the value of \`${s.key}\` must be a list`);
  }
  if (s.options && !s.options.includes(text)) throw bad(`the value of \`${s.key}\` must be one of: ${s.options.join(", ")}`);
  return text;
}

export const consolePluginHandlers = [
  http.get(A, () => {
    requireAdmin();
    return HttpResponse.json({ items: installs.map(view) });
  }),
  http.get(`${A}/:id`, ({ params }) => {
    requireAdmin();
    return HttpResponse.json(view(find(String(params.id))));
  }),
  http.put(`${A}/:id/status`, async ({ params, request }) => {
    requireAdmin();
    const i = find(String(params.id));
    i.status = ((await request.json()) as { status: "enabled" | "disabled" }).status;
    return HttpResponse.json(view(i));
  }),
  http.put(`${A}/:id/grants`, async ({ params, request }) => {
    requireAdmin();
    const i = find(String(params.id));
    const b = (await request.json()) as { scopes: string[]; all_courses: boolean; course_ids: string[] };
    const extra = b.scopes.find((s) => !i.requested_scopes.includes(s));
    if (extra) throw problem(422, "Validation failed", undefined, [{ field: "scopes", message: `the plugin did not ask for the scope \`${extra}\`` }]);
    i.scopes = b.scopes;
    i.all_courses = b.all_courses;
    i.course_ids = b.course_ids;
    return HttpResponse.json(view(i));
  }),
  http.get(`${A}/:id/settings`, ({ params }) => {
    requireAdmin();
    const i = find(String(params.id));
    const items: AdminPluginSetting[] = i.specs.map((s) => ({
      ...s,
      is_set: isSet(i, s),
      // A secret never carries its value.
      value: s.secret || !isSet(i, s) ? (s.secret ? null : s.default) : (JSON.parse(i.values[s.key] ?? "null") as AdminPluginSetting["value"]),
    }));
    return HttpResponse.json({ items });
  }),
  http.put(`${A}/:id/settings/:key`, async ({ params, request }) => {
    requireAdmin();
    const i = find(String(params.id));
    const s = i.specs.find((x) => x.key === params.key);
    if (!s) throw problem(404, "Not found");
    const text = ((await request.json()) as { value: string }).value;
    i.values[s.key] = JSON.stringify(parse(s, text));
    return new HttpResponse(null, { status: 204 });
  }),
  http.delete(`${A}/:id/settings/:key`, ({ params }) => {
    requireAdmin();
    const i = find(String(params.id));
    if (!i.specs.some((x) => x.key === params.key)) throw problem(404, "Not found");
    delete i.values[String(params.key)];
    return new HttpResponse(null, { status: 204 });
  }),
  http.get(`${A}/:id/calls`, ({ params }) => {
    requireAdmin();
    const i = find(String(params.id));
    return HttpResponse.json({
      counts: i.calls,
      recent: i.calls.map((c) => ({ at: MOCK_NOW.toISOString(), method: c.method, route: c.route, status: c.status, duration_ms: c.average_ms })),
    });
  }),
];
