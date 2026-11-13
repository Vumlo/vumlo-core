import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD } from "./routes";

// An administrator finds every course on the courses page, narrowed by the server, and opens any of them. The
// mock's sample courses: five in Autumn 2026 (the current term), the archived ones in older terms.

async function signIn(page: Page, email: string) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
}

test("an admin finds any course by term, status and text, and opens it", async ({ page }) => {
  await signIn(page, accounts.admin);
  await page.goto("/courses");
  await page.getByRole("tab", { name: "All courses" }).click();

  // The current term first.
  await expect(page.getByRole("link", { name: "Databases" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Programming Fundamentals" })).toHaveCount(0);

  // A text matches the owner's surname, whatever the case.
  await page.getByRole("searchbox", { name: "Search by code, title or owner" }).fill("VOSS");
  await expect(page.getByRole("link", { name: "Graph Algorithms" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Databases" })).toHaveCount(0);

  // Every term, archived only.
  await page.getByLabel("Term", { exact: true }).selectOption({ label: "All terms" });
  await page.getByLabel("Status", { exact: true }).selectOption({ label: "Archived" });
  await expect(page.getByRole("link", { name: "Programming Fundamentals" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Graph Algorithms" })).toHaveCount(0);

  // The admin is not on the roster and opens it all the same.
  await page.getByRole("link", { name: "Programming Fundamentals" }).click();
  await expect(page).toHaveURL(/\/course\?id=/);
  await expect(page.getByRole("heading", { name: "Programming Fundamentals" }).first()).toBeVisible();
});

test("the admin course page keeps the terms and no table of courses", async ({ page }) => {
  await signIn(page, accounts.admin);
  await page.goto("/admin/courses");
  await expect(page.getByRole("heading", { name: "Terms", exact: true }).first()).toBeVisible();
  await expect(page.getByText("Autumn 2026").first()).toBeVisible();
  await expect(page.getByRole("table")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Change owner" })).toHaveCount(0);
});
