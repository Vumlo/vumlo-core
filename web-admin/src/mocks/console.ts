import { http, HttpResponse } from "msw";
import { MOCK_NOW } from "@/lib/clock";
import { getDb } from "@/mocks/db";
import { problem, publicUser, SESSION_KEY } from "@/mocks/logic";

const A = "/api/v1";

// The admin console's own sign-in (ADR-0095). The mock keeps its session per origin (localStorage),
// so the console on another address is signed in apart from the student app, as in the real thing.
// Only an administrator signs in; anyone else gets the answer of a wrong password.
export const consoleHandlers = [
  http.post(`${A}/console/login`, async ({ request }) => {
    const body = (await request.json()) as { email?: string; password?: string };
    const u = getDb().users.find((x) => x.email === (body.email ?? "").trim().toLowerCase());
    if (!u || u.status !== "active" || u.role !== "admin" || !u.password || u.password !== body.password) throw problem(401, "Unauthorized");
    localStorage.setItem(SESSION_KEY, u.id);
    u.last_sign_in = MOCK_NOW.toISOString();
    return HttpResponse.json(publicUser(u), { headers: { "Cache-Control": "no-store" } });
  }),
];
