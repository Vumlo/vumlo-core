// The administration pages open in both themes, on a phone and in Ukrainian (ADR-0095).
import { expect, test, type ConsoleMessage, type Page } from "@playwright/test";
import { PASSWORD, accounts, store } from "./support";

const PAGES = ["/", "/people", "/groups", "/terms", "/units", "/plugins", "/mail", "/institution", "/audit"];

function watch(page: Page) {
  const problems: string[] = [];
  page.on("console", (m: ConsoleMessage) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  return problems;
}

async function signIn(page: Page, labels = { email: "Email", password: "Password", submit: "Sign in" }) {
  await page.goto("/login");
  await page.getByLabel(labels.email).fill(accounts.admin);
  await page.getByLabel(labels.password, { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: labels.submit, exact: true }).click();
  await page.waitForURL((u) => u.pathname !== "/login");
}

async function open(page: Page, path: string, theme: string, problems: string[]) {
  // The mock's world lives in the page, so the session is kept in storage and a load starts from the seed again.
  await page.goto(path);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.locator("main h1").first()).toBeVisible();
  await page.waitForLoadState("networkidle");
  await expect(page.getByText("Something went wrong")).toHaveCount(0);
  expect(problems, `console problems on ${path}`).toEqual([]);
}

for (const theme of ["light", "dark"] as const) {
  test(`every page opens in the ${theme} theme`, async ({ page }) => {
    await store(page, "vumlo.theme", theme);
    const problems = watch(page);
    await signIn(page);
    for (const path of PAGES) await open(page, path, theme, problems);
  });
}

test("every page fits a phone without sideways scrolling", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  const problems = watch(page);
  await signIn(page);
  for (const path of PAGES) {
    await open(page, path, "light", problems);
    expect(await page.evaluate(() => document.documentElement.scrollWidth), `page width on ${path}`).toBeLessThanOrEqual(390);
  }
});

test("every page opens in Ukrainian", async ({ page }) => {
  await store(page, "vumlo.language", "uk");
  const problems = watch(page);
  await signIn(page, { email: "Електронна пошта", password: "Пароль", submit: "Увійти" });
  for (const path of PAGES) await open(page, path, "light", problems);
  await expect(page.locator("html")).toHaveAttribute("lang", "uk");
});

test("the sidebar names every page, and the terms page has the terms and no table of courses", async ({ page }) => {
  await signIn(page);
  const side = page.getByRole("navigation", { name: "Admin console" });
  for (const name of ["People", "Groups", "Terms", "Units and roles", "Plugins", "Mail", "Institution", "Audit log"]) {
    await expect(side.getByRole("link", { name })).toBeVisible();
  }
  await side.getByRole("link", { name: "Terms" }).click();
  await expect(page).toHaveURL("/terms");
  await expect(page.getByText("Autumn 2026").first()).toBeVisible();
  await expect(page.getByRole("table")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Change owner" })).toHaveCount(0);
});

test("an address that is no page says so inside the frame", async ({ page }) => {
  await signIn(page);
  await page.getByRole("navigation", { name: "Admin console" }).getByRole("link", { name: "People" }).click();
  await page.evaluate(() => window.history.pushState(null, "", "/no-such-page"));
  await page.reload();
  await expect(page.getByText("There is no such page in the admin console.")).toBeVisible();
});
