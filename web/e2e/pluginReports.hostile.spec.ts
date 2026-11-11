import { expect, test, type Page } from "@playwright/test";
import { courseHref } from "../src/lib/routes";
import { accounts, PASSWORD, store } from "./routes";

// The "Plugin reports" panel with reports a hostile or broken plugin could write (src/mocks/handlers/pluginReports.ts,
// `hostile`, switched on by the localStorage key below): markup, bad links, a huge table, huge strings, nested garbage.
// The panel must show text as text, drop what is not allowed, stay bounded and not throw.

async function open(page: Page) {
  const problems: string[] = [];
  const dialogs: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  page.on("dialog", (d) => {
    dialogs.push(d.message());
    void d.dismiss();
  });
  await store(page, "vumlo.mock.hostileReports", "1");
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.teacher);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  const course = await page.evaluate(async () => {
    const r = await fetch("/api/v1/me/courses?limit=100");
    return ((await r.json()).items as { id: string }[])[0]!.id;
  });
  await page.goto(courseHref(course));
  const panel = page.getByRole("region", { name: "Plugin reports" });
  await expect(panel.getByRole("article", { name: "Big" })).toBeVisible();
  return { panel, problems, dialogs };
}

test("markup in a report is text; script and foreign links are not links", async ({ page }) => {
  const { panel, problems, dialogs } = await open(page);
  const xss = '<script>window.__xss = 1</script><img src=x onerror="window.__xss = 2">';
  const article = panel.getByRole("article", { name: xss });
  await expect(article).toBeVisible();
  // Seven places show the markup as typed: heading, table caption, summary, column header, first cell, fact label and value.
  await expect(article.getByText(xss, { exact: true })).toHaveCount(7);
  await expect(article.getByRole("definition").first()).toHaveText(xss);
  expect(await article.locator("script, img, iframe, object, embed").count()).toBe(0);
  expect(await panel.locator("script, img, iframe, object, embed").count()).toBe(0);

  // Negative numbers follow the language.
  await expect(article.getByRole("cell", { name: "-12.5" }).or(article.getByRole("rowheader", { name: "-12.5" }))).toBeVisible();
  await expect(article.getByText("-1,234.5")).toBeVisible();

  // Of four links only the plugin's own one stays, and nothing points at javascript:.
  await expect(article.getByRole("link")).toHaveCount(1);
  await expect(article.getByRole("link", { name: "Mine" })).toHaveAttribute("href", "/api/v1/plugins/hostile-html/export");
  expect(await panel.locator('a[href^="javascript:" i], a[href^="//"]').count()).toBe(0);

  expect(await page.evaluate(() => (window as unknown as { __xss?: number }).__xss)).toBeUndefined();
  expect(dialogs).toEqual([]);
  expect(problems).toEqual([]);
});

test("a 10 000 row table and huge strings stay bounded", async ({ page }) => {
  const { panel, problems } = await open(page);
  const big = panel.getByRole("article", { name: "Big" });
  await expect(big.getByRole("table").getByRole("row")).toHaveCount(201); // the header and 200 rows
  await expect(big.getByText("Showing the first 200 of 10,000 rows.")).toBeVisible();
  // The 300 000 character summary and the 500 000 character cell are cut.
  const summary = (await big.locator("p").filter({ hasText: /^s{100}/ }).first().textContent()) ?? "";
  expect(summary.length).toBeLessThanOrEqual(10_001);
  expect(summary.endsWith("…")).toBe(true);
  const cell = (await big.getByRole("cell", { name: /^w{100}/ }).first().textContent()) ?? "";
  expect(cell.length).toBeLessThanOrEqual(2_001);
  expect(cell.endsWith("…")).toBe(true);
  expect(problems).toEqual([]);
});

test("nested garbage in display falls back to the summary and a cut raw view", async ({ page }) => {
  const { panel, problems } = await open(page);
  const deep = panel.getByRole("article", { name: "hostile-deep · deep" });
  await expect(deep.getByText("Nested garbage.")).toBeVisible();
  await expect(deep.getByRole("table")).toHaveCount(0);
  await expect(deep.getByRole("definition")).toHaveCount(0);
  await deep.getByText("Raw data").click();
  const raw = (await deep.locator("pre").textContent()) ?? "";
  expect(raw.length).toBeGreaterThan(0);
  expect(raw.length).toBeLessThanOrEqual(20_001);
  expect(problems).toEqual([]);
});
