// The admin console is the same app on another address (ADR-0095): the host decides which door answers.
// The student app is `localhost`, the console is `127.0.0.1` (the dev server is started with
// VUMLO_ADMIN_HOST for it, see playwright.config.ts). Browsers keep the two apart: no shared cookies,
// and the mock API keeps its session per origin.
import { expect, test, type Page } from "@playwright/test";
import { PASSWORD, accounts } from "./routes";

const APP = process.env.E2E_BASE_URL ?? "http://localhost:3200";
const CONSOLE = process.env.E2E_CONSOLE_URL ?? "http://127.0.0.1:3200";

async function signIn(page: Page, email = accounts.admin, password = PASSWORD) {
  await page.goto(`${CONSOLE}/console/login`);
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
}

test("the admin host serves only the console", async ({ page }) => {
  for (const path of ["/", "/courses", "/calendar", "/admin/people"]) {
    await page.goto(`${CONSOLE}${path}`);
    await expect(page, path).toHaveURL(/\/console\/login/);
  }
  // The API is the same on both addresses: the host rule never answers a call with a redirect.
  const api = await page.request.get(`${CONSOLE}/api/v1/health`, { maxRedirects: 0 });
  expect([301, 302, 307, 308]).not.toContain(api.status());
});

test("the app host has no console", async ({ page }) => {
  for (const path of ["/console", "/console/login", "/console/people"]) {
    const response = await page.goto(`${APP}${path}`);
    expect(response?.status(), path).toBe(404);
  }
});

test("a console sign-in reaches the console and sign out returns to the sign-in", async ({ page }) => {
  await signIn(page);
  await expect(page).toHaveURL(`${CONSOLE}/console`);
  await expect(page.getByRole("heading", { name: "Admin console" })).toBeVisible();
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/console\/login/);
  await page.goto(`${CONSOLE}/console`);
  await expect(page).toHaveURL(/\/console\/login/);
});

test("a wrong password is explained and a non-admin cannot sign in", async ({ page }) => {
  await signIn(page, accounts.admin, "not the password");
  await expect(page.locator(".alert-danger")).toContainText("don't match");
  await expect(page).toHaveURL(/\/console\/login/);
  await signIn(page, accounts.teacher, PASSWORD);
  await expect(page.locator(".alert-danger")).toContainText("don't match");
});

test("a 401 on a console call goes to the console sign-in, not the students'", async ({ page }) => {
  await signIn(page);
  await expect(page).toHaveURL(`${CONSOLE}/console`);
  await page.evaluate(() => localStorage.removeItem("vumlo.mock.session"));
  await page.reload();
  await expect(page).toHaveURL(/\/console\/login/);
  expect(new URL(page.url()).pathname).not.toBe("/login");
});

test("a temporary password sends the administrator to change it first", async ({ page }) => {
  await signIn(page);
  await expect(page).toHaveURL(`${CONSOLE}/console`);
  // Another administrator resets this account's password: the next sign-in carries the flag.
  const temporary = await page.evaluate(async () => {
    const me = await (await fetch("/api/v1/me")).json();
    const reset = await (await fetch(`/api/v1/admin/users/${me.id}/reset-password`, { method: "POST" })).json();
    return reset.temporary_password as string;
  });
  // The mock's world lives in this page, so sign out and in again without a reload.
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/console\/login/);
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(temporary);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/console\/change-password/);
  await page.getByLabel("Temporary password").fill(temporary);
  await page.getByLabel("New password", { exact: true }).fill("a much longer passphrase 1");
  await page.getByLabel("Repeat new password").fill("a much longer passphrase 1");
  await page.getByRole("button", { name: "Save and continue" }).click();
  await expect(page).toHaveURL(`${CONSOLE}/console`);
});
