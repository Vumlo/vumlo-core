// The mail queue and journal in the console (ADR-0087, ADR-0095). The mock has mail configured (SMTP): one message
// waiting, two sent, and three dead (two of the plugin `ocr`, one of the core). No address is ever on the page.
import { expect, test, type Page } from "@playwright/test";
import { PASSWORD, accounts } from "./support";

async function open(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => u.pathname !== "/login");
  await page.getByRole("navigation", { name: "Admin console" }).getByRole("link", { name: "Mail" }).click();
}

test("the page shows the queue counts and the newest messages", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Mail", exact: true })).toBeVisible();
  const counts = page.getByRole("region", { name: "Last 24 hours" });
  await expect(counts.getByText("Waiting", { exact: true }).locator("..")).toContainText("1");
  await expect(counts.getByText("Sent", { exact: true }).locator("..")).toContainText("2");
  await expect(counts.getByText("Dead", { exact: true }).locator("..")).toContainText("3");
  await expect(page.getByRole("table", { name: "Messages" }).getByRole("row")).toHaveCount(7);
});

test("a filter by state and plugin narrows the journal", async ({ page }) => {
  await open(page);
  const table = page.getByRole("table", { name: "Messages" });
  await page.getByLabel("State").selectOption("dead");
  await expect(table.getByRole("row")).toHaveCount(4);
  await page.getByLabel("Plugin").fill("ocr");
  await expect(table.getByRole("row")).toHaveCount(3);
});

test("no address appears on the page", async ({ page }) => {
  await open(page);
  await expect(page.locator("main")).not.toContainText("@");
});

test("retrying dead mail asks how many, and reports how many were put back and how many could not be", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Retry dead mail" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("3");
  await dialog.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByRole("status").filter({ hasText: "put back" })).toContainText("2 put back");
  await expect(page.getByRole("status").filter({ hasText: "put back" })).toContainText("1 could not be");
});

test("without mail configured the page says so", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("vumlo.mock.mail", "off"));
  await open(page);
  await expect(page.getByText("Mail is not set up")).toBeVisible();
});
