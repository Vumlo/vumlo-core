// The console's own sign-in (ADR-0095). The mock keeps its session in this origin's storage, so the console is
// signed in apart from the student app, as it is by its own cookie in the real thing.
import { expect, test, type Page } from "@playwright/test";
import { PASSWORD, accounts } from "./support";

async function signIn(page: Page, email: string = accounts.admin, password: string = PASSWORD) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
}

test("without a session every page leads to the console's sign-in", async ({ page }) => {
  for (const path of ["/", "/people", "/plugins", "/audit", "/no-such-page"]) {
    await page.goto(path);
    await expect(page, path).toHaveURL(/\/login/);
  }
});

test("a sign-in reaches the console and sign out returns to the sign-in", async ({ page }) => {
  await signIn(page);
  await expect(page).toHaveURL("/");
  await expect(page.getByRole("heading", { name: "Admin console" })).toBeVisible();
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/login/);
  await page.goto("/");
  await expect(page).toHaveURL(/\/login/);
});

test("a wrong password is explained, and so is the password of someone who is not an administrator", async ({ page }) => {
  await signIn(page, accounts.admin, "not the password");
  await expect(page.locator(".alert-danger")).toContainText("don't match");
  await expect(page).toHaveURL(/\/login/);
  await signIn(page, accounts.teacher, PASSWORD);
  await expect(page.locator(".alert-danger")).toContainText("don't match");
});

test("the page asked for is the page after signing in, and nowhere else", async ({ page }) => {
  await page.goto("/plugins");
  await expect(page).toHaveURL(/\/login\?next=%2Fplugins/);
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page).toHaveURL("/plugins");
  // An address outside the console is not followed.
  await page.getByRole("button", { name: "Sign out" }).click();
  await page.goto("/login?next=//evil.example/x");
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page).toHaveURL("/");
});

test("a 401 on a console call goes to the console sign-in", async ({ page }) => {
  await signIn(page);
  await expect(page).toHaveURL("/");
  await page.evaluate(() => localStorage.removeItem("vumlo.mock.session"));
  await page.reload();
  await expect(page).toHaveURL(/\/login/);
});

test("a temporary password sends the administrator to change it first", async ({ page }) => {
  await signIn(page);
  await expect(page).toHaveURL("/");
  // Another administrator resets this account's password: the next sign-in carries the flag.
  const temporary = await page.evaluate(async () => {
    const me = await (await fetch("/api/v1/me")).json();
    const reset = await (await fetch(`/api/v1/admin/users/${me.id}/reset-password`, { method: "POST" })).json();
    return reset.temporary_password as string;
  });
  // The mock's world lives in this page, so sign out and in again without a reload.
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/login/);
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(temporary);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page).toHaveURL(/\/change-password/);
  await page.getByLabel("Temporary password").fill(temporary);
  await page.getByLabel("New password", { exact: true }).fill("a much longer passphrase 1");
  await page.getByLabel("Repeat new password").fill("a much longer passphrase 1");
  await page.getByRole("button", { name: "Save and continue" }).click();
  await expect(page).toHaveURL("/");
});
