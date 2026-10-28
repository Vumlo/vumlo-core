import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD, type RoleName } from "./routes";

async function signIn(page: Page, role: RoleName) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts[role]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  await expect(page.locator("header").first()).toBeVisible();
}

/** The first course of the signed-in person, from the API. */
async function firstCourse(page: Page): Promise<string> {
  return page.evaluate(async () => (await (await fetch("/api/v1/me/courses?limit=100")).json()).items[0].id as string);
}

test("the grading queue opens the grade of the student it names", async ({ page }) => {
  await signIn(page, "teacher");
  await page.goto("/grading");
  const first = page.getByRole("row").filter({ has: page.getByRole("link", { name: "Grade", exact: true }) }).first();
  await expect(first).toBeVisible();
  const student = (await first.locator("span[role=cell]").first().locator("span").last().innerText()).trim();
  await first.getByRole("link", { name: "Grade", exact: true }).click();
  await expect(page.locator("#student-h")).toHaveText(student);
});

test("publishing all draft grades asks first", async ({ page }) => {
  await signIn(page, "teacher");
  await page.goto("/grading");
  const publish = page.getByRole("button", { name: /^Publish \d+/ }).first();
  await expect(publish).toBeVisible();
  await publish.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Students see");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
});

test("a row menu on the content page closes on Escape and on a click outside", async ({ page }) => {
  await signIn(page, "teacher");
  await page.goto(`/course?id=${await firstCourse(page)}`);
  const more = page.getByRole("button", { name: /^More actions for / }).first();
  await more.click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await more.click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.getByRole("heading", { level: 1 }).first().click();
  await expect(page.getByRole("menu")).toHaveCount(0);
});

test("an item can be moved from its menu, without dragging", async ({ page }) => {
  await signIn(page, "teacher");
  await page.goto(`/course?id=${await firstCourse(page)}`);
  const items = page.locator("section[aria-label] [data-kind]");
  await expect(items.first()).toBeVisible();
  const before = await items.allInnerTexts();
  expect(before.length).toBeGreaterThan(1);
  await items.first().getByRole("button", { name: /^More actions for / }).click();
  await page.getByRole("menuitem", { name: "Move down" }).click();
  await expect.poll(async () => (await items.allInnerTexts())[0]).not.toBe(before[0]);
});

test("the account menu opens on its first entry and Escape returns to the button", async ({ page }) => {
  await signIn(page, "student");
  const button = page.getByRole("button", { name: /^Account menu/ });
  await button.click();
  await expect(page.getByRole("menuitem").first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem").nth(1)).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(button).toBeFocused();
});

test("the page search box is the only one on the search screen", async ({ page }) => {
  await signIn(page, "student");
  await page.goto("/search?q=merge");
  await expect(page.getByRole("search")).toHaveCount(1);
});

test("submitting a quiz with unanswered questions on its last attempt asks first", async ({ page }) => {
  await signIn(page, "student");
  const href = await page.evaluate(async () => {
    const courses = await (await fetch("/api/v1/me/courses?limit=100")).json();
    for (const c of courses.items) {
      const tree = await (await fetch(`/api/v1/courses/${c.id}/content`)).json();
      const items = tree.modules.flatMap((m: { items: { id: string; title: string }[]; submodules: { items: { id: string; title: string }[] }[] }) => [...m.items, ...m.submodules.flatMap((s) => s.items)]);
      const hit = items.find((i: { title: string }) => i.title === "Sorting quiz");
      if (hit) return `/course/item?id=${c.id}&item=${hit.id}`;
    }
    return null;
  });
  expect(href, "the quiz with one attempt").toBeTruthy();
  await page.goto(href!);
  await page.getByRole("button", { name: "Submit quiz" }).last().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("unanswered");
  await expect(dialog).toContainText("last attempt");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Submit quiz" }).first()).toBeVisible();
});

test("the new assignment dialog explains each type, checks the title and opens the editor", async ({ page }) => {
  await signIn(page, "teacher");
  await page.goto(`/course?id=${await firstCourse(page)}`);
  await page.getByRole("button", { name: "Assignment" }).first().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("radio")).toHaveCount(4);
  await expect(dialog.getByRole("radio", { name: /Text answer/ })).toHaveAttribute("aria-checked", "true");
  // A blank title is explained in place, and nothing is sent.
  await dialog.getByRole("button", { name: "Create and edit" }).click();
  await expect(dialog.getByText("Give it a title.")).toBeVisible();
  await expect(dialog).toBeVisible();
  // The arrow keys change the type, and a form has no points.
  await dialog.getByRole("radio", { name: /Text answer/ }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(dialog.getByRole("radio", { name: /Quiz/ })).toHaveAttribute("aria-checked", "true");
  await dialog.getByRole("button", { name: "20", exact: true }).click();
  await expect(dialog.getByLabel("Points")).toHaveValue("20");
  await dialog.getByRole("radio", { name: /Form/ }).click();
  await expect(dialog.getByLabel("Points")).toHaveCount(0);
  await dialog.getByRole("radio", { name: /Quiz/ }).click();
  // Enter in the title field creates it.
  await dialog.getByLabel("Title").fill("Quiz on sorting");
  await dialog.getByLabel("Title").press("Enter");
  await page.waitForURL(/\/course\/item\/edit/);
});
