// Administration is the admin console's, on another address (ADR-0095): the student app has no admin pages, and an
// administrator reaches the console by a link in their account menu.
import { expect, test } from "@playwright/test";
import { PASSWORD, accounts } from "./routes";

test("the student app has no admin routes", async ({ page }) => {
  for (const path of ["/admin", "/admin/people", "/admin/groups", "/admin/courses", "/admin/institution", "/admin/audit", "/console"]) {
    const response = await page.goto(path);
    expect(response?.status(), path).toBe(404);
  }
});

test("an administrator has an Admin console link in the account menu and no Admin item in the navigation", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
  await expect(page).toHaveURL("/courses");
  await expect(page.getByRole("navigation", { name: "Main" }).first().getByRole("link", { name: "Admin" })).toHaveCount(0);
  await page.getByRole("button", { name: /^Account menu/ }).click();
  const link = page.getByRole("menuitem", { name: "Admin console" });
  await expect(link).toBeVisible();
  expect(await link.getAttribute("href")).toMatch(/^https?:\/\/[^/]+\/?$/);
});

test("a teacher has no Admin console link", async ({ page }) => {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.teacher);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
  await page.getByRole("button", { name: /^Account menu/ }).click();
  await expect(page.getByRole("menuitem", { name: "Admin console" })).toHaveCount(0);
});
