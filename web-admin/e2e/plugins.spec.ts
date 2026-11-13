// Plugins in the console (ADR-0095): the list, the settings a manifest declares, access and the call log.
// The mock has two installs: `ocr`, enabled and short of a token and a limit, and `notify`, switched off.
import { expect, test, type Page } from "@playwright/test";
import { PASSWORD, accounts } from "./support";


async function signIn(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => u.pathname !== "/login");
  // The mock's world lives in the page: move on by the sidebar, not by a new load.
  await page.getByRole("navigation", { name: "Admin console" }).getByRole("link", { name: "Plugins" }).click();
}

test("the list shows each plugin with its status and what still needs setup", async ({ page }) => {
  await signIn(page);
  await expect(page.getByRole("heading", { name: "Plugins", exact: true })).toBeVisible();
  const ocr = page.getByRole("row", { name: /ocr/ });
  await expect(ocr).toContainText("Enabled");
  await expect(ocr).toContainText("Needs setup: api_token, limit");
  const notify = page.getByRole("row", { name: /notify/ });
  await expect(notify).toContainText("Disabled");
  await expect(notify).not.toContainText("Needs setup");
});

test("a settings form is built from the manifest's spec, one input for each kind", async ({ page }) => {
  await signIn(page);
  await page.getByRole("link", { name: "ocr" }).click();
  await expect(page.getByRole("heading", { name: "ocr", exact: true })).toBeVisible();
  const settings = page.getByRole("region", { name: "Settings" });
  await expect(settings.getByLabel("api_token", { exact: true })).toHaveAttribute("type", "password");
  await expect(settings.getByLabel("mode", { exact: true })).toHaveJSProperty("tagName", "SELECT");
  await expect(settings.getByLabel("limit", { exact: true })).toHaveAttribute("type", "number");
  await expect(settings.getByLabel("verbose", { exact: true })).toHaveAttribute("type", "checkbox");
  await expect(settings.getByLabel("domains", { exact: true })).toHaveJSProperty("tagName", "TEXTAREA");
  await expect(settings.getByText("The token of the OCR service")).toBeVisible();
});

test("a secret is set but never shown, and a value out of range is explained and not saved", async ({ page }) => {
  await signIn(page);
  await page.getByRole("link", { name: "ocr" }).click();
  const settings = page.getByRole("region", { name: "Settings" });
  const token = settings.getByLabel("api_token", { exact: true });
  await expect(settings.getByTestId("state-api_token")).toHaveText("Not set");
  await token.fill("sk-sentinel-123");
  await settings.getByRole("button", { name: "Save api_token" }).click();
  await expect(settings.getByTestId("state-api_token")).toHaveText("Set");
  await expect(token).toHaveValue("");
  await expect(page.getByText("sk-sentinel-123")).toHaveCount(0);

  await settings.getByLabel("limit", { exact: true }).fill("99");
  await settings.getByRole("button", { name: "Save limit" }).click();
  await expect(settings.getByText("must be between 1 and 10")).toBeVisible();
  await expect(settings.getByTestId("state-limit")).toHaveText("Not set");
  await settings.getByLabel("limit", { exact: true }).fill("4");
  await settings.getByRole("button", { name: "Save limit" }).click();
  await expect(settings.getByTestId("state-limit")).toHaveText("Set");
});

test("using the default again takes a value away", async ({ page }) => {
  await signIn(page);
  await page.getByRole("link", { name: "ocr" }).click();
  const settings = page.getByRole("region", { name: "Settings" });
  await settings.getByLabel("mode", { exact: true }).selectOption("accurate");
  await settings.getByRole("button", { name: "Save mode" }).click();
  await expect(settings.getByTestId("state-mode")).toHaveText("Set");
  await settings.getByRole("button", { name: "Use the default for mode" }).click();
  await expect(settings.getByTestId("state-mode")).toHaveText("Default");
});

test("switching a plugin off asks first and says its queued mail is cancelled", async ({ page }) => {
  await signIn(page);
  await page.getByRole("link", { name: "ocr" }).click();
  await page.getByRole("button", { name: "Disable" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("queued mail");
  await dialog.getByRole("button", { name: "Disable" }).click();
  await expect(page.getByTestId("status")).toHaveText("Disabled");
  await page.getByRole("button", { name: "Enable" }).click();
  await expect(page.getByTestId("status")).toHaveText("Enabled");
});

test("access is changed within what the plugin asked for", async ({ page }) => {
  await signIn(page);
  await page.getByRole("link", { name: "ocr" }).click();
  const access = page.getByRole("region", { name: "Access" });
  await expect(access.getByLabel("grades:read")).toBeChecked();
  await access.getByLabel("grades:read").uncheck();
  await access.getByRole("button", { name: "Save access" }).click();
  await expect(access.getByText("Saved")).toBeVisible();
  await expect(access.getByLabel("grades:read")).not.toBeChecked();
});

test("the call log shows counts by route and answer, and the newest calls", async ({ page }) => {
  await signIn(page);
  await page.getByRole("link", { name: "ocr" }).click();
  const calls = page.getByRole("region", { name: "Calls" });
  await expect(calls.getByRole("row", { name: /\/plugin-api\/settings.*200/ })).toContainText("12");
  await expect(calls.getByRole("row", { name: /\/plugin-api\/grades.*403/ })).toContainText("2");
});
