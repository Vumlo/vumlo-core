import { expect, test, type ConsoleMessage, type Page } from "@playwright/test";
import { itemHref } from "../src/lib/routes";
import { accounts, PASSWORD, store, type RoleName } from "./routes";

// The page of a plugin in a tab of an assignment (ADR-0066), against the mock plugin "words"
// (public/mock-plugin/tab.html). What is checked is the boundary: the frame is sandboxed, gets its
// context only from this page, and cannot reach it.

type Found = { courseId: string; assignment: string; lecture: string };

async function signIn(page: Page, role: RoleName) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts[role]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
}

/** An assignment and a lecture the signed-in person can open, from the API like the other specs. */
async function findItems(page: Page): Promise<Found> {
  return page.evaluate(async () => {
    type Item = { id: string; kind: string };
    type Tree = { modules: { items: Item[]; submodules: { items: Item[] }[] }[] };
    const get = async (p: string) => (await fetch(`/api/v1${p}`)).json();
    const courses = (await get("/me/courses?limit=100")).items as { id: string }[];
    for (const { id } of courses) {
      const tree = (await get(`/courses/${id}/content`)) as Tree;
      const items = tree.modules.flatMap((m) => [...m.items, ...m.submodules.flatMap((s) => s.items)]);
      const assignment = items.find((i) => i.kind === "assignment");
      const lecture = items.find((i) => i.kind === "text");
      if (assignment && lecture) return { courseId: id, assignment: assignment.id, lecture: lecture.id };
    }
    throw new Error("no course with an assignment and a lecture");
  });
}

function watch(page: Page) {
  const problems: string[] = [];
  page.on("console", (m: ConsoleMessage) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  return problems;
}

const frameOf = (page: Page) => page.locator('iframe[title="Words, a plugin"]');

for (const role of ["student", "teacher"] as const) {
  for (const theme of ["light", "dark"] as const) {
    test(`${role}: the plugin tab opens a sandboxed frame that gets its context (${theme})`, async ({ page }) => {
      await store(page, "vumlo.theme", theme);
      const problems = watch(page);
      await signIn(page, role);
      const { courseId, assignment } = await findItems(page);

      await page.goto(itemHref(courseId, assignment));
      const tabs = page.getByRole("navigation", { name: "Views of this assignment" });
      await expect(tabs.getByRole("link", { name: "Assignment" })).toHaveAttribute("aria-current", "page");
      await tabs.getByRole("link", { name: "Words" }).click();
      await expect(page).toHaveURL(new RegExp(`tab=words`));
      await expect(tabs.getByRole("link", { name: "Words" })).toHaveAttribute("aria-current", "page");

      // The frame: sandboxed to scripts only, no referrer, no permissions, and the address the core gave.
      const frame = frameOf(page);
      await expect(frame).toHaveAttribute("sandbox", "allow-scripts");
      await expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
      await expect(frame).toHaveAttribute("allow", "");
      await expect(frame).toHaveAttribute("src", "/mock-plugin/tab.html");

      // It was told what it needs, and the colours of the current theme.
      const inside = page.frameLocator('iframe[title="Words, a plugin"]');
      await expect(inside.locator("#state")).toHaveText("Context received.");
      await expect(inside.locator("#assignment")).toHaveText(assignment);
      await expect(inside.locator("#role")).toHaveText(role === "student" ? "student" : "staff");
      await expect(inside.locator("#theme")).toHaveText(theme);
      await expect(inside.locator("#tokens")).toContainText("15 tokens");
      // Its own request for a height was taken.
      await expect(frame).toHaveCSS("height", "300px");

      // And it is shut in: it cannot read the page around it or its cookies.
      const reach = await frame.elementHandle().then((h) => h!.contentFrame()).then((f) =>
        f!.evaluate(() => {
          const attempt = (read: () => unknown) => {
            try {
              read();
              return "readable";
            } catch {
              return "blocked";
            }
          };
          return { parent: attempt(() => (parent as Window).document.title), cookie: attempt(() => document.cookie), storage: attempt(() => localStorage.length) };
        }),
      );
      expect(reach).toEqual({ parent: "blocked", cookie: "blocked", storage: "blocked" });

      // The assignment itself is one click back, and nothing was logged as an error.
      await tabs.getByRole("link", { name: "Assignment" }).click();
      await expect(frame).toHaveCount(0);
      expect(problems).toEqual([]);
    });
  }
}

test("a message that does not come from the frame is ignored", async ({ page }) => {
  await signIn(page, "teacher");
  const { courseId, assignment } = await findItems(page);
  await page.goto(itemHref(courseId, assignment, "words"));
  const frame = frameOf(page);
  await expect(frame).toHaveCSS("height", "300px");

  // This page posts to itself: `event.source` is this window, not the frame's.
  await page.evaluate(() => window.postMessage({ type: "vumlo.resize", v: 1, height: 900 }, "*"));
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  await expect(frame).toHaveCSS("height", "300px");
});

test("a change of theme is sent to the frame again", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await signIn(page, "student");
  const { courseId, assignment } = await findItems(page);
  await page.goto(itemHref(courseId, assignment, "words"));
  const inside = page.frameLocator('iframe[title="Words, a plugin"]');
  await expect(inside.locator("#theme")).toHaveText("light");
  await expect(inside.locator("#contexts")).toHaveText("1");

  await page.emulateMedia({ colorScheme: "dark" });

  await expect(inside.locator("#theme")).toHaveText("dark");
  await expect(inside.locator("#contexts")).toHaveText("2");
});

test("a lecture has no plugin tab", async ({ page }) => {
  await signIn(page, "teacher");
  const { courseId, lecture } = await findItems(page);
  await page.goto(itemHref(courseId, lecture));
  await expect(page.locator("h1, h2").first()).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Views of this assignment" })).toHaveCount(0);
});
