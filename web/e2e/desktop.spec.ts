import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD } from "./routes";

// The desktop app's window (ADR-0060) without the app: a stand-in for what Tauri puts on `window`, so the
// banner, the queue notices and the sign-out question can be seen with the mock API behind them. The real
// commands are tested in desktop/ (cargo test).
const standIn = `
  const callbacks = new Map();
  const listeners = {};
  let n = 1;
  let state = { online: true, pending: 0, failed: 0, paused: false, last_ok: Date.now(), sent: 0, queued: [] };
  window.__fake = {
    calls: [],
    outbox: [],
    route: null,
    ping(name) { for (const id of listeners[name] ?? []) callbacks.get(id)({ event: name, id: 0, payload: null }); },
    emit(partial) {
      state = { ...state, ...partial };
      for (const id of listeners.sync ?? []) callbacks.get(id)({ event: "sync", id: 0, payload: state });
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    transformCallback(cb) { const id = n++; callbacks.set(id, cb); return id; },
    unregisterCallback(id) { callbacks.delete(id); },
    convertFileSrc: (p) => p,
    async invoke(cmd, args) {
      window.__fake.calls.push({ cmd, args });
      if (cmd === "plugin:event|listen") { (listeners[args.event] ??= []).push(args.handler); return n++; }
      if (cmd === "sync_state") return state;
      if (cmd === "outbox_list") return window.__fake.outbox;
      if (cmd === "sync_now" || cmd === "outbox_discard" || cmd === "set_language" || cmd === "plugin:event|unlisten") return null;
      if (cmd === "take_route") { const r = window.__fake.route; window.__fake.route = null; return r; }
      if (cmd === "api") {
        const res = await fetch("/api/v1" + args.path, {
          method: args.method,
          headers: args.body ? { "content-type": "application/json" } : undefined,
          body: args.body ? JSON.stringify(args.body) : undefined,
        });
        const text = res.status === 204 ? "" : await res.text();
        let body = text ? JSON.parse(text) : null;
        if (args.path === "/auth/login" && res.ok && body && body.user) body = body.user;
        return { status: res.status, body, from_cache: false, cached_at: null, queued: false };
      }
      throw new Error("unknown command " + cmd);
    },
  };
`;

async function signedIn(page: Page, email = accounts.student) {
  await page.addInitScript(standIn);
  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
}

const emit = (page: Page, partial: Record<string, unknown>) => page.evaluate((p) => (window as unknown as { __fake: { emit: (x: unknown) => void } }).__fake.emit(p), partial);
const calls = (page: Page) => page.evaluate(() => (window as unknown as { __fake: { calls: { cmd: string; args: unknown }[] } }).__fake.calls);

test("a browser shows nothing of the desktop app", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.student);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
  await expect(page.getByText("You are offline")).toHaveCount(0);
});

test("going offline says so, shows what waits, and 'try now' asks the app", async ({ page }) => {
  await signedIn(page);
  await emit(page, { online: false, pending: 2 });
  await expect(page.getByRole("status").filter({ hasText: "You are offline" })).toContainText("2 changes are waiting to be sent.");
  await expect(page.getByRole("status").filter({ hasText: "You are offline" })).toContainText("Last updated");
  await page.getByRole("button", { name: "Try now" }).click();
  expect((await calls(page)).some((c) => c.cmd === "sync_now")).toBe(true);
  await emit(page, { online: true, pending: 0 });
  await expect(page.getByText("You are offline")).toHaveCount(0);
});

test("the app reads what the screens ask for through the app, with the session of the app", async ({ page }) => {
  await signedIn(page);
  const used = await calls(page);
  expect(used.some((c) => c.cmd === "api" && (c.args as { path: string }).path === "/me")).toBe(true);
  const login = used.find((c) => c.cmd === "api" && (c.args as { path: string }).path === "/auth/login");
  expect((login!.args as { body: { client: string } }).body.client).toBe("app");
});

test("what the server refused is shown in its words and can be discarded", async ({ page }) => {
  await signedIn(page);
  await page.evaluate(() => {
    (window as unknown as { __fake: { outbox: unknown[] } }).__fake.outbox = [
      { id: 7, method: "POST", path: "/assignments/a1/submission/submit", body: null, state: "failed", status: 409, problem: JSON.stringify({ detail: "the submission window is closed" }), attempts: 1, created_at: 0 },
      { id: 8, method: "PUT", path: "/assignments/a2/submission/draft", body: JSON.stringify({ content: { v: 1, blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "my words" }] }] } }), state: "failed", status: 409, problem: JSON.stringify({ detail: "the submission window is closed" }), attempts: 1, created_at: 0 },
    ];
  });
  await emit(page, { failed: 2 });
  await expect(page.getByRole("alert").filter({ hasText: "2 changes were not accepted" })).toBeVisible();
  await page.getByRole("button", { name: "Review" }).click();
  await expect(page.getByText("Submission", { exact: true })).toBeVisible();
  await expect(page.getByText("Draft of an assignment")).toBeVisible();
  await expect(page.getByText("The server said: the submission window is closed")).toHaveCount(2);
  await page.getByRole("button", { name: "Discard" }).first().click();
  expect((await calls(page)).some((c) => c.cmd === "outbox_discard" && (c.args as { id: number }).id === 7)).toBe(true);
});

test("a session that ended holds the queue and says what to do", async ({ page }) => {
  await signedIn(page);
  await emit(page, { paused: true, pending: 1 });
  await expect(page.getByRole("alert").filter({ hasText: "Your session has ended" })).toBeVisible();
});

test("signing out with changes that were not sent asks first", async ({ page }) => {
  await signedIn(page);
  await emit(page, { pending: 1 });
  await page.getByRole("button", { name: /^Account menu/ }).click();
  await page.getByRole("menuitem", { name: "Sign out" }).click();
  await expect(page.getByRole("heading", { name: "Sign out with changes that were not sent?" })).toBeVisible();
  await expect(page.getByText("1 change has not been sent. If you sign out, it is deleted.")).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page).not.toHaveURL(/\/login/);
  await page.getByRole("button", { name: /^Account menu/ }).click();
  await page.getByRole("menuitem", { name: "Sign out" }).click();
  await page.getByRole("button", { name: "Sign out anyway" }).click();
  await expect(page).toHaveURL(/\/login/);
});

test("a quiz needs a connection", async ({ page }) => {
  await signedIn(page);
  const work = await page.evaluate(async () => (await (await fetch("/api/v1/me/work")).json()) as { items: { assignment_id: string; course_id: string; type: string; status: string }[] });
  const quiz = work.items.find((i) => i.type === "quiz" && (i.status === "not_started" || i.status === "draft"));
  test.skip(!quiz, "the mock has no open quiz for this student");
  await page.goto(`/course/item?id=${quiz!.course_id}&item=${quiz!.assignment_id}`);
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await expect(page.getByText("A quiz needs a connection")).toHaveCount(0);
  await emit(page, { online: false });
  await expect(page.getByText("A quiz needs a connection. Connect to the internet to start or continue it.")).toBeVisible();
  await emit(page, { online: true });
  await expect(page.getByText("A quiz needs a connection")).toHaveCount(0);
});

test("a submit that waits in the queue says so and holds the work", async ({ page }) => {
  await signedIn(page);
  const work = await page.evaluate(async () => (await (await fetch("/api/v1/me/work")).json()) as { items: { assignment_id: string; course_id: string; type: string; status: string }[] });
  const task = work.items.find((i) => i.type !== "quiz" && i.type !== "form" && (i.status === "not_started" || i.status === "draft"));
  test.skip(!task, "the mock has no open assignment for this student");
  await page.goto(`/course/item?id=${task!.course_id}&item=${task!.assignment_id}`);
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await emit(page, { pending: 1, queued: [`/assignments/${task!.assignment_id}/submission/submit`] });
  await expect(page.getByText("Your work is saved on this device and will be sent when you are online.")).toBeVisible();
});

test("a link from outside opens the screen it names", async ({ page }) => {
  await signedIn(page);
  await page.evaluate(() => {
    const fake = (window as unknown as { __fake: { route: string | null; ping: (name: string) => void } }).__fake;
    fake.route = "/calendar";
    fake.ping("navigate");
  });
  await expect(page).toHaveURL(/\/calendar/);
});

test("the app is told the language of the window", async ({ page }) => {
  await signedIn(page);
  await expect.poll(async () => (await calls(page)).filter((c) => c.cmd === "set_language").map((c) => (c.args as { language: string }).language)).toContain("en");
});
