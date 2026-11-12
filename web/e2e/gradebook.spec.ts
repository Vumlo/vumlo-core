import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD, type RoleName } from "./routes";
import { courseHref } from "../src/lib/routes";

async function signIn(page: Page, role: RoleName) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts[role]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  await expect(page.locator("header").first()).toBeVisible();
}

const firstCourse = (page: Page) => page.evaluate(async () => (await (await fetch("/api/v1/me/courses?limit=100")).json()).items[0].id as string);

test("the gradebook shows a total per student and opens the Structure mode", async ({ page }) => {
  await signIn(page, "teacher");
  await page.goto(courseHref(await firstCourse(page), "grades"));
  await expect(page.getByRole("columnheader", { name: /^Total/ })).toBeVisible();
  await expect(page.getByRole("link", { name: "Download Excel" })).toHaveAttribute("href", /\/gradebook\/export\.xlsx$/);
  const modes = page.getByRole("group", { name: "Gradebook mode" });
  await expect(modes.getByRole("button", { name: "Grade" })).toHaveAttribute("aria-pressed", "true");
  await modes.getByRole("button", { name: "Structure" }).click();
  await expect(page.getByRole("heading", { name: "Total", exact: true })).toBeVisible();
  await expect(page.getByLabel("Target points")).toHaveValue("100");
  await expect(page.getByLabel("Pass mark")).toHaveValue("60");
  await page.getByLabel("Pass mark").fill("50");
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(page.locator(".alert-danger")).toHaveCount(0);
  await expect(page.getByLabel("Pass mark")).toHaveValue("50");
});

test("a student sees a total of the course", async ({ page }) => {
  await signIn(page, "student");
  await page.goto(courseHref(await firstCourse(page), "grades"));
  await expect(page.getByText("Your total")).toBeVisible();
});
