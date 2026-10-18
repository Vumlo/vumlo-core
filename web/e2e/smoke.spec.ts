import { expect, test, type ConsoleMessage, type Page } from "@playwright/test";
import { accounts, courseRoutes, PASSWORD, publicRoutes, staticRoutes, type RoleName } from "./routes";

const roles: RoleName[] = ["student", "teacher", "admin"];
const themes = ["light", "dark"] as const;

function watch(page: Page) {
  const problems: string[] = [];
  page.on("console", (m: ConsoleMessage) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  return problems;
}

async function signIn(page: Page, role: RoleName) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts[role]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.locator("header, nav").first()).toBeVisible();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
}

async function open(page: Page, path: string, theme: string, problems: string[]) {
  await page.goto(path);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.locator("h1, h2, main [contenteditable], main input").first()).toBeVisible();
  await page.waitForLoadState("networkidle");
  await expect(page.getByText("Something went wrong")).toHaveCount(0);
  expect(problems, `console problems on ${path}`).toEqual([]);
}

for (const role of roles) {
  for (const theme of themes) {
    test(`${role} screens open in ${theme} theme`, async ({ page }) => {
      await page.addInitScript((t) => localStorage.setItem("vumlo.theme", t), theme);
      const problems = watch(page);
      await signIn(page, role);
      const routes = [...staticRoutes[role], ...(await courseRoutes(page, role))];
      expect(routes.length).toBeGreaterThan(5);
      for (const path of routes) await open(page, path, theme, problems);
    });
  }
}

for (const role of roles) {
  test(`${role} screens fit a phone without sideways scrolling`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 800 });
    const problems = watch(page);
    await signIn(page, role);
    const routes = [...staticRoutes[role], ...(await courseRoutes(page, role))];
    for (const path of routes) {
      await open(page, path, "light", problems);
      const width = await page.evaluate(() => document.documentElement.scrollWidth);
      expect(width, `page width on ${path}`).toBeLessThanOrEqual(390);
    }
  });
}

for (const role of roles) {
  test(`${role} screens open in Ukrainian`, async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem("vumlo.language", "uk"));
    const problems = watch(page);
    await page.goto("/login");
    await page.getByLabel("Електронна пошта").fill(accounts[role]);
    await page.getByLabel("Пароль", { exact: true }).fill(PASSWORD);
    await page.getByRole("button", { name: "Увійти", exact: true }).click();
    await page.waitForURL((u) => !u.pathname.startsWith("/login"));
    const routes = [...staticRoutes[role], ...(await courseRoutes(page, role))];
    for (const path of routes) await open(page, path, "light", problems);
    await expect(page.locator("html")).toHaveAttribute("lang", "uk");
  });
}

for (const theme of themes) {
  test(`pages before sign-in open in ${theme} theme`, async ({ page }) => {
    await page.addInitScript((t) => localStorage.setItem("vumlo.theme", t), theme);
    const problems = watch(page);
    for (const path of publicRoutes) await open(page, path, theme, problems);
  });
}

test("a wrong password shows an inline message", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.student);
  await page.getByLabel("Password", { exact: true }).fill("not the password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByText("That email and password don't match")).toBeVisible();
});

test("phones get bottom tabs", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  await signIn(page, "student");
  await expect(page.getByRole("navigation", { name: /main|tabs|primary/i }).last()).toBeVisible();
});
