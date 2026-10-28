import { expect, test, type Page } from "@playwright/test";
import { accounts, courseRoutes, PASSWORD } from "./routes";

async function openLectureEditor(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.teacher);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  const edit = (await courseRoutes(page, "teacher")).find((r) => r.startsWith("/course/item/edit"));
  expect(edit, "an item to edit").toBeTruthy();
  await page.goto(edit!);
  const last = page.locator("[data-editable]").last();
  await last.click();
  await page.keyboard.press("Control+End");
  await page.keyboard.press("Enter");
}

/** The student's draft answer of "Merge sort implementation": a paragraph and a code block, which is last. */
async function openAnswer(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.student);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  const href = await page.evaluate(async () => {
    const courses = await (await fetch("/api/v1/me/courses?limit=100")).json();
    for (const c of courses.items) {
      const tree = await (await fetch(`/api/v1/courses/${c.id}/content`)).json();
      const items = tree.modules.flatMap((m: { items: { id: string; title: string }[]; submodules: { items: { id: string; title: string }[] }[] }) => [...m.items, ...m.submodules.flatMap((s) => s.items)]);
      const hit = items.find((i: { title: string }) => i.title === "Merge sort implementation");
      if (hit) return `/course/item?id=${c.id}&item=${hit.id}`;
    }
    return null;
  });
  expect(href, "the assignment with a draft").toBeTruthy();
  await page.goto(href!);
  await expect(page.locator("[data-editable]").first()).toBeVisible();
}

/** A click on the empty page under the last block of the answer. */
async function clickBelowLastBlock(page: Page) {
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  const box = (await page.getByRole("group", { name: "Your answer" }).boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height - 16);
}

test("a click below the last block goes on writing there, even after a code block", async ({ page }) => {
  await openAnswer(page);
  const blocks = page.locator("[data-block-id]");
  const before = await blocks.count();
  await clickBelowLastBlock(page);
  await page.keyboard.type("and then more");
  await expect(blocks).toHaveCount(before + 1);
  await expect(page.locator("[data-editable]").last()).toHaveText("and then more");
});

test("a formula is drawn by KaTeX, and Enter leaves the formula field for the text after it", async ({ page }) => {
  await openAnswer(page);
  await clickBelowLastBlock(page);
  await page.keyboard.type("/formula");
  await page.keyboard.press("Enter");
  await page.keyboard.type("\\int_{1}^{2} x \\, dx");
  await expect(page.locator(".math .katex-html").last()).toContainText("∫");
  await page.keyboard.press("Enter");
  await page.keyboard.type("after");
  await expect(page.locator("[data-editable]").filter({ hasText: "after" })).toHaveCount(1);
});

test("the block menu opened with the + of the gutter is filtered by what is typed", async ({ page }) => {
  await openAnswer(page);
  const first = page.locator("[data-editable]").first();
  await first.hover();
  await page.getByRole("button", { name: "Add a block" }).first().click({ force: true });
  await expect(page.getByRole("listbox")).toBeVisible();
  await page.keyboard.type("numbered");
  await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(1);
  await page.keyboard.press("Enter");
  await expect(page.locator("[data-block-id]").getByText("1.", { exact: true })).toBeVisible();
});

test("typing / opens the block menu at once", async ({ page }) => {
  await openLectureEditor(page);
  await page.keyboard.type("/");
  await expect(page.getByRole("listbox")).toBeVisible();
  await page.keyboard.type(" ");
  await expect(page.getByRole("listbox")).toHaveCount(0);
});

test("a list marker typed as a shortcut is not kept as text", async ({ page }) => {
  await openLectureEditor(page);
  await page.keyboard.type("- a point");
  const item = page.locator("[data-editable]").last();
  await expect(item).toHaveText("a point");
  await expect(page.locator("[data-block-id]").last().getByText("•")).toBeVisible();
});
