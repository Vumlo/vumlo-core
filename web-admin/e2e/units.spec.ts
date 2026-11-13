// Units and unit roles in the console (ADR-0094, ADR-0095). The mock has Faculty of Science (with two courses and the
// department Computer Science inside it) and Faculty of Arts; Elena Voss is a manager of Computer Science.
import { expect, test, type Page } from "@playwright/test";
import { PASSWORD, accounts } from "./support";

async function open(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.admin);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => u.pathname !== "/login");
  // The mock's world lives in the page: move on by the sidebar, not by a new load.
  await page.getByRole("navigation", { name: "Admin console" }).getByRole("link", { name: "Units and roles" }).click();
}

const tree = (page: Page) => page.getByRole("tree", { name: "Units" });

test("the units are a tree, each inside the one it belongs to", async ({ page }) => {
  await open(page);
  await expect(tree(page).getByRole("treeitem", { name: "Faculty of Science" })).toHaveAttribute("aria-level", "1");
  await expect(tree(page).getByRole("treeitem", { name: "Computer Science" })).toHaveAttribute("aria-level", "2");
  await expect(tree(page).getByRole("treeitem", { name: "Faculty of Arts" })).toHaveAttribute("aria-level", "1");
});

test("an admin creates, renames and moves a unit", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "New top-level unit" }).click();
  await page.getByRole("dialog").getByLabel("Name").fill("Faculty of Law");
  await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();
  await expect(tree(page).getByRole("treeitem", { name: "Faculty of Law" })).toHaveAttribute("aria-level", "1");

  await tree(page).getByRole("treeitem", { name: "Faculty of Law" }).click();
  await page.getByRole("button", { name: "Rename" }).click();
  await page.getByRole("dialog").getByLabel("Name").fill("School of Law");
  await page.getByRole("dialog").getByRole("button", { name: "Save" }).click();
  await expect(tree(page).getByRole("treeitem", { name: "School of Law" })).toBeVisible();

  await page.getByRole("button", { name: "Move" }).click();
  await page.getByRole("dialog").getByLabel("Inside").selectOption({ label: "Faculty of Arts" });
  await page.getByRole("dialog").getByRole("button", { name: "Save" }).click();
  await expect(tree(page).getByRole("treeitem", { name: "School of Law" })).toHaveAttribute("aria-level", "2");
});

test("a name that is taken under the same parent is explained", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "New top-level unit" }).click();
  await page.getByRole("dialog").getByLabel("Name").fill("Faculty of Arts");
  await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText("already");
});

test("a unit with courses or units inside is not deleted, and one without is", async ({ page }) => {
  await open(page);
  await tree(page).getByRole("treeitem", { name: "Faculty of Science" }).click();
  await page.getByRole("button", { name: "Delete" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Delete" }).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText("still has");
  await page.getByRole("dialog").getByRole("button", { name: "Cancel" }).click();

  await tree(page).getByRole("treeitem", { name: "Faculty of Arts" }).click();
  await page.getByRole("button", { name: "Delete" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Delete" }).click();
  await expect(tree(page).getByRole("treeitem", { name: "Faculty of Arts" })).toHaveCount(0);
});

test("roles are added by search and removed", async ({ page }) => {
  await open(page);
  await tree(page).getByRole("treeitem", { name: "Computer Science" }).click();
  const roles = page.getByRole("region", { name: "People with a role on Computer Science" });
  await expect(roles.getByRole("row", { name: /Voss/ })).toContainText("Manager");

  await roles.getByRole("searchbox", { name: "Search teachers and administrators by name or email" }).fill("okafor");
  await expect(roles.getByText("Nobody matches.")).toBeVisible();
  await roles.getByRole("searchbox", { name: "Search teachers and administrators by name or email" }).fill("kovalska");
  await roles.getByRole("button", { name: /^Add Marta/ }).click();
  await expect(roles.getByRole("row", { name: /Kovalska/ })).toContainText("Viewer");

  await roles.getByRole("button", { name: "Remove Marta Kovalska" }).click();
  await expect(roles.getByRole("row", { name: /Kovalska/ })).toHaveCount(0);
});

test("an ability switch changes the role for every unit, and says so", async ({ page }) => {
  await open(page);
  const abilities = page.getByRole("region", { name: "What each role may do" });
  await expect(abilities).toContainText("every unit");
  const viewerGrade = abilities.getByRole("checkbox", { name: "Viewer: Grade" });
  await expect(viewerGrade).not.toBeChecked();
  await viewerGrade.click();
  await expect(abilities.getByText("Saved. Applies to every unit.")).toBeVisible();
  await expect(viewerGrade).toBeChecked();
});
