import { http, HttpResponse } from "msw";
import { MOCK_NOW } from "@/lib/clock";
import { getDb, uid, type DbUser } from "../db";
import { problem, publicUser, SESSION_KEY } from "../logic";

const A = "/api/v1";
/** The mock institution only accepts addresses from this domain (ADR-0003). */
const ALLOWED_DOMAIN = "northfield.edu";

const field = (f: string, message: string) => problem(422, "Validation failed", undefined, [{ field: f, message }]);
const clean = (v: unknown) => (typeof v === "string" ? v.trim() : "");

function token(kind: string) {
  const db = getDb();
  return uid(`${kind}:${db.mail.length}:${db.users.length}:${Date.now()}`);
}

function send(to: string, kind: "verify" | "reset", path: string) {
  const db = getDb();
  db.mail.unshift({ id: uid(`mail:${db.mail.length}:${to}`), to, kind, path, sent_at: MOCK_NOW.toISOString() });
}

function openSession(u: DbUser) {
  localStorage.setItem(SESSION_KEY, u.id);
  u.last_sign_in = MOCK_NOW.toISOString();
}

function newStudent(name: string, surname: string, email: string, password: string): DbUser {
  const db = getDb();
  const u: DbUser = {
    id: uid(`user:self:${db.users.length}:${email}`), email, name, surname, role: "student", status: "active", must_change_password: false,
    created_at: MOCK_NOW.toISOString(), password, last_sign_in: null,
  };
  db.users.push(u);
  return u;
}

export const authHandlers = [
  // Self-registration with a password. The answer is the same whether or not the address has an account (ADR-0017).
  http.post(`${A}/auth/register`, async ({ request }) => {
    const b = (await request.json()) as Record<string, unknown>;
    const name = clean(b.name);
    const surname = clean(b.surname);
    const email = clean(b.email).toLowerCase();
    const password = typeof b.password === "string" ? b.password : "";
    const errors = [
      ...(name ? [] : [{ field: "name", message: "Enter your first name" }]),
      ...(surname ? [] : [{ field: "surname", message: "Enter your last name" }]),
      ...(/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email) ? (email.endsWith(`@${ALLOWED_DOMAIN}`) ? [] : [{ field: "email", message: `Use your university email, ending in @${ALLOWED_DOMAIN}` }]) : [{ field: "email", message: "Enter a valid email address" }]),
      ...(password.length >= 10 ? [] : [{ field: "password", message: "Use at least 10 characters" }]),
    ];
    if (errors.length) throw problem(422, "Validation failed", undefined, errors);
    const db = getDb();
    if (!db.users.some((u) => u.email === email)) {
      const t = token("verify");
      db.signups = db.signups.filter((s) => s.email !== email);
      db.signups.push({ token: t, name, surname, email, password });
      send(email, "verify", `/verify-email?token=${t}`);
    }
    return HttpResponse.json({ sent: true }, { status: 202 });
  }),
  http.post(`${A}/auth/verify-email`, async ({ request }) => {
    const b = (await request.json()) as { token?: string };
    const db = getDb();
    const pending = db.signups.find((s) => s.token === b.token);
    if (!pending) throw problem(410, "Gone", "This link has been used or has expired");
    db.signups = db.signups.filter((s) => s !== pending);
    db.mail = db.mail.filter((m) => !m.path.endsWith(pending.token));
    if (!db.users.some((u) => u.email === pending.email)) newStudent(pending.name, pending.surname, pending.email, pending.password);
    return new HttpResponse(null, { status: 204 });
  }),

  // Password reset by email. Same answer for every address.
  http.post(`${A}/auth/password-reset`, async ({ request }) => {
    const b = (await request.json()) as { email?: string };
    const email = clean(b.email).toLowerCase();
    if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) throw field("email", "Enter a valid email address");
    const db = getDb();
    const u = db.users.find((x) => x.email === email && x.status === "active");
    if (u) {
      const t = token("reset");
      db.resets = db.resets.filter((r) => r.user_id !== u.id);
      db.resets.push({ token: t, user_id: u.id });
      send(email, "reset", `/reset-password?token=${t}`);
    }
    return HttpResponse.json({ sent: true }, { status: 202 });
  }),
  http.post(`${A}/auth/password-reset/confirm`, async ({ request }) => {
    const b = (await request.json()) as { token?: string; new_password?: string };
    const db = getDb();
    const r = db.resets.find((x) => x.token === b.token);
    if (!r) throw problem(410, "Gone", "This link has been used or has expired");
    if ((b.new_password ?? "").length < 10) throw field("new_password", "Use at least 10 characters");
    const u = db.users.find((x) => x.id === r.user_id)!;
    u.password = b.new_password!;
    u.must_change_password = false;
    db.resets = db.resets.filter((x) => x !== r);
    db.mail = db.mail.filter((m) => !m.path.endsWith(r.token));
    return new HttpResponse(null, { status: 204 });
  }),

  // Google sign-in (ADR-0017, path 3). A real server answers with Google's consent URL; the mock skips the consent screen.
  http.post(`${A}/auth/google/start`, async ({ request }) => {
    const b = (await request.json()) as { login_hint?: string };
    const q = new URLSearchParams({ code: "mock", state: "mock", hint: clean(b.login_hint).toLowerCase() });
    return HttpResponse.json({ authorization_url: `/auth/google/callback?${q}` });
  }),
  http.post(`${A}/auth/google/callback`, async ({ request }) => {
    const b = (await request.json()) as { code?: string; hint?: string };
    // A sign-in the desktop app started ends with a code for the app, not with a session (ADR-0062).
    if (b.code === "mock-app") return HttpResponse.json({ handoff: "mock-handoff" }, { headers: { "Cache-Control": "no-store" } });
    if (b.code !== "mock") throw problem(401, "Unauthorized");
    const db = getDb();
    const email = clean(b.hint).toLowerCase() || "maya.okafor@northfield.edu";
    let u = db.users.find((x) => x.email === email);
    if (u && u.status !== "active") throw problem(403, "Forbidden", "This account is disabled");
    if (!u) {
      if (!email.endsWith(`@${ALLOWED_DOMAIN}`)) throw problem(403, "Forbidden", `Only @${ALLOWED_DOMAIN} Google accounts can sign in here`);
      const [first = "New", last = "Student"] = email.split("@")[0]!.split(".");
      const cap = (w: string) => w.charAt(0).toUpperCase() + w.slice(1);
      u = newStudent(cap(first), cap(last), email, "");  // no password: this account signs in with Google only
    }
    openSession(u);
    return HttpResponse.json(publicUser(u), { headers: { "Cache-Control": "no-store" } });
  }),

  // Mock only: the mailbox of the fake mail server.
  http.get(`${A}/mock/outbox`, () => HttpResponse.json({ items: getDb().mail })),
];
