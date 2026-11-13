// The administration pages are the console's (ADR-0095): they open there in both themes, on a phone and in Ukrainian,
// and the student app has none of them. The console is `127.0.0.1`, the student app `localhost` (see console-host.spec.ts).
import { expect, test, type ConsoleMessage, type Page } from "@playwright/test";
import { PASSWORD, accounts, store } from "./routes";

const APP = process.env.E2E_BASE_URL ?? "http://localhost:3200";
const CONSOLE = process.env.E2E_CONSOLE_URL ?? "http://127.0.0.1:3200";

const PAGES = ["/console", "/console/people", "/console/groups", "/console/terms", "/console/institution", "/console/audit"];

function watch(page: Page) {
  const problems: string[] = [];
  page.on("console", (m: ConsoleMessage) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  return problems;
}

async function signIn(page: Page, labels = { email: "Email", password: "Password", submit: "Sign in" }) {
  await page.goto(`${CONSOLE}/console/login`);
  await page.getByLabel(labels.email).fill(accounts.admin);
  await page.getByLabel(labels.password, { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: labels.submit, exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/console/login"));
}

async function open(page: Page, path: string, theme: string, problems: string[]) {
  const response = await page.goto(`${CONSOLE}${path}`);
  expect(response?.status(), `status of ${path}`).toBe(200);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.locator("main h1").first()).toBeVisible();
  await page.waitForLoadState("networkidle");
  await expect(page.getByText("Something went wrong")).toHaveCount(0);
  expect(problems, `console problems on ${path}`).toEqual([]);
}

for (const theme of ["light", "dark"] as const) {
  test(`every console page opens in the ${theme} theme`, async ({ page }) => {
    await store(page, "vumlo.theme", theme);
    const problems = watch(page);
    await signIn(page);
    for (const path of PAGES) await open(page, path, theme, problems);
  });
}

test("every console page fits a phone without sideways scrolling", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  const problems = watch(page);
  await signIn(page);
  for (const path of PAGES) {
    await open(page, path, "light", problems);
    expect(await page.evaluate(() => document.documentElement.scrollWidth), `page width on ${path}`).toBeLessThanOrEqual(390);
  }
});

test("every console page opens in Ukrainian", async ({ page }) => {
  await store(page, "vumlo.language", "uk");
  const problems = watch(page);
  await signIn(page, { email: "Електронна пошта", password: "Пароль", submit: "Увійти" });
  for (const path of PAGES) await open(page, path, "light", problems);
  await expect(page.locator("html")).toHaveAttribute("lang", "uk");
});

test("the sidebar names every page and the terms page keeps the terms and no table of courses", async ({ page }) => {
  await signIn(page);
  const side = page.getByRole("navigation", { name: "Admin console" });
  for (const name of ["People", "Groups", "Terms", "Institution", "Audit log"]) {
    await expect(side.getByRole("link", { name })).toBeVisible();
  }
  await side.getByRole("link", { name: "Terms" }).click();
  await expect(page).toHaveURL(`${CONSOLE}/console/terms`);
  await expect(page.getByText("Autumn 2026").first()).toBeVisible();
  await expect(page.getByRole("table")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Change owner" })).toHaveCount(0);
});

test("the student app has no admin routes", async ({ page }) => {
  for (const path of ["/admin", "/admin/people", "/admin/groups", "/admin/courses", "/admin/institution", "/admin/audit"]) {
    const response = await page.goto(`${APP}${path}`);
    expect(response?.status(), path).toBe(404);
  }
});

test("an administrator has an Admin console link in the account menu and no Admin item in the navigation", async ({ page }) => {
  await page.goto(`${APP}/login`);
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
  await expect(page).toHaveURL(`${APP}/courses`);
  await expect(page.getByRole("navigation", { name: "Main" }).first().getByRole("link", { name: "Admin" })).toHaveCount(0);
  await page.getByRole("button", { name: /^Account menu/ }).click();
  const link = page.getByRole("menuitem", { name: "Admin console" });
  await expect(link).toBeVisible();
  expect(await link.getAttribute("href")).toMatch(/^https?:\/\/[^/]+\/console$/);
});
