import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD, type RoleName } from "./routes";
import { accounts as seeded } from "../src/mocks/identities";

// Org units in the people's app (ADR-0094). The mock's world: Faculty of Science with the department Computer Science inside
// it. Grace Lin (the dean) manages Computer Science and teaches nothing there; Hanna Kravets (the viewer) views the whole
// faculty. Databases sits in Computer Science and Linear Algebra in the faculty itself; neither is taught by either of them,
// its teacher is Marta Kowalska ("kowalska"), who holds no unit role.

async function signIn(page: Page, who: RoleName | "kowalska") {
  await page.goto("/login");
  await page.getByLabel("Email").fill(who === "kowalska" ? seeded.kowalska : accounts[who]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
}

const main = (page: Page) => page.getByRole("navigation", { name: "Main" }).first();
const unit = (page: Page, name: string) => page.getByRole("treeitem", { name });
const tab = (page: Page, name: string) => page.getByRole("tab", { name, exact: true });
const save = (page: Page) => page.getByRole("button", { name: /^Save/ }).first();
const courseId = (page: Page, title: string) =>
  page.evaluate(async (t) => ((await (await fetch("/api/v1/me/courses?limit=100")).json()).items as { id: string; title: string }[]).find((c) => c.title === t)!.id, title);

// The mock's world lives in the page, so a test moves on by the links and not by a new load.
async function openUnits(page: Page) {
  await main(page).getByRole("link", { name: "Units", exact: true }).click();
  await page.waitForURL(/\/units/);
}

async function newCourse(page: Page, title: string, unitName: string) {
  await page.goto("/courses/new");
  await page.getByLabel("Course title").fill(title);
  await page.getByLabel("Unit", { exact: true }).selectOption({ label: unitName });
  await page.getByRole("button", { name: /Create/ }).click();
  await page.waitForURL(/\/course\/settings/);
}

test("a dean sees Units, opens the unit and a course they do not teach, with its gradebook", async ({ page }) => {
  await signIn(page, "dean");
  await openUnits(page);
  await expect(unit(page, "Computer Science")).toBeVisible();
  await expect(unit(page, "Faculty of Science")).toHaveCount(0);
  await unit(page, "Computer Science").click();
  await page.getByRole("link", { name: /Databases/ }).click();

  await tab(page, "Gradebook").click();
  await expect(page.getByRole("columnheader", { name: /^Total/ })).toBeVisible();

  // The manager default holds manage_courses, so the settings are open and show the unit.
  await tab(page, "Settings").click();
  await expect(page.getByLabel("Unit", { exact: true }).locator("option:checked")).toHaveText("Computer Science");
  // They may place courses in their own unit, so the picker offers a choice.
  await expect(page.getByLabel("Unit", { exact: true })).toBeEnabled();
  await expect(page.getByLabel("Unit", { exact: true }).getByRole("option")).toHaveCount(2);

  // It lacks edit_content: the content is shown and nothing on it changes.
  await tab(page, "Content").click();
  await expect(page.getByRole("heading", { name: "Databases" }).first()).toBeVisible();
  await page.waitForLoadState("networkidle");
  await expect(page.getByRole("button", { name: /add section/i })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Edit", exact: true })).toHaveCount(0);
});

test("a dean creates a course in their unit", async ({ page }) => {
  await signIn(page, "dean");
  await newCourse(page, "Dean's New Course", "Computer Science");
  await openUnits(page);
  await unit(page, "Computer Science").click();
  await expect(page.getByRole("link", { name: /Dean's New Course/ })).toBeVisible();
  await expect(page.getByRole("link", { name: /Databases/ })).toBeVisible();
});

test("a course moves to No unit and back", async ({ page }) => {
  await signIn(page, "dean");
  await newCourse(page, "Roaming Course", "Computer Science");
  const unitSelect = page.getByLabel("Unit", { exact: true });

  // The dean owns it, so it stays theirs when it leaves the unit.
  await unitSelect.selectOption({ label: "No unit" });
  await save(page).click();
  await expect(page.getByRole("status").filter({ hasText: /Saved/ })).toBeVisible();
  await openUnits(page);
  await unit(page, "Computer Science").click();
  await expect(page.getByRole("link", { name: /Databases/ })).toBeVisible();
  await expect(page.getByRole("link", { name: /Roaming Course/ })).toHaveCount(0);

  await page.getByRole("link", { name: "Courses", exact: true }).first().click();
  await page.getByRole("link", { name: /Roaming Course/ }).first().click();
  await tab(page, "Settings").click();
  await unitSelect.selectOption({ label: "Computer Science" });
  await save(page).click();
  await expect(page.getByRole("status").filter({ hasText: /Saved/ })).toBeVisible();
  await openUnits(page);
  await unit(page, "Computer Science").click();
  await expect(page.getByRole("link", { name: /Roaming Course/ })).toBeVisible();
});

test("a viewer sees grades and no edit control", async ({ page }) => {
  await signIn(page, "viewer");
  await openUnits(page);
  await expect(unit(page, "Faculty of Science")).toBeVisible();
  await unit(page, "Computer Science").click();
  await page.getByRole("link", { name: /Databases/ }).click();
  await expect(page.getByRole("heading", { name: "Databases" }).first()).toBeVisible();
  await page.waitForLoadState("networkidle");
  await expect(page.getByRole("button", { name: /add section/i })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Edit", exact: true })).toHaveCount(0);
  await expect(tab(page, "Settings")).toHaveCount(0);

  await tab(page, "Gradebook").click();
  await expect(page.getByRole("columnheader", { name: /^Total/ })).toBeVisible();
  await expect(page.getByRole("link", { name: "Download Excel" })).toBeVisible();
  // Grading and the structure need `grade`: a viewer has no mode switch, no structure save and no publishing.
  await expect(page.getByRole("group", { name: "Gradebook mode" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Structure", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Save settings" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Publish .*draft/i })).toHaveCount(0);

  // A cell opens the work to read; nothing on it saves a grade.
  await page.getByRole("gridcell").getByRole("button").first().click();
  await expect(page).toHaveURL(/\/course\/grade\?/);
  await expect(page.getByRole("region", { name: "Submission" })).toBeVisible();
  await expect(page.getByRole("heading", { level: 2 }).first()).toBeVisible();
  for (const name of ["Save and next", "Save draft", "Publish"]) await expect(page.getByRole("button", { name, exact: true })).toHaveCount(0);
  await expect(page.getByLabel("Points")).toHaveCount(0);
  await page.goBack();

  // A course of the faculty itself opens the same way.
  await openUnits(page);
  await unit(page, "Faculty of Science").click();
  await page.getByRole("link", { name: /Linear Algebra/ }).click();
  await tab(page, "Gradebook").click();
  await expect(page.getByRole("columnheader", { name: /^Total/ })).toBeVisible();
  await expect(tab(page, "Settings")).toHaveCount(0);
});

for (const role of ["student", "teacher"] as const) {
  test(`a ${role} without a unit role has no Units link and no unit select`, async ({ page }) => {
    await signIn(page, role);
    await page.waitForLoadState("networkidle");
    await expect(main(page).getByRole("link", { name: "Units", exact: true })).toHaveCount(0);
    await page.goto("/courses/new");
    await page.waitForLoadState("networkidle");
    await expect(page.getByLabel("Unit", { exact: true })).toHaveCount(0);
  });
}

test("a dean manages the units below theirs", async ({ page }) => {
  await signIn(page, "dean");
  await openUnits(page);
  await expect(page.getByRole("button", { name: "New top-level unit" })).toHaveCount(0);
  await unit(page, "Computer Science").click();
  const dialog = page.getByRole("dialog");
  const roles = page.getByRole("region", { name: /People with a role on/ });

  // The unit the dean's role sits on is changed from above: they add below it and change nothing of it.
  await expect(page.getByRole("button", { name: "New unit inside", exact: true })).toBeVisible();
  for (const name of ["Rename", "Move", "Delete"]) await expect(page.getByRole("button", { name, exact: true })).toHaveCount(0);
  await expect(roles).toHaveCount(0);

  await page.getByRole("button", { name: "New unit inside", exact: true }).click();
  await dialog.getByLabel("Name").fill("Databases Lab");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(unit(page, "Databases Lab")).toHaveAttribute("aria-level", "2");

  await unit(page, "Databases Lab").click();
  for (const name of ["Rename", "Move", "Delete"]) await expect(page.getByRole("button", { name, exact: true })).toBeVisible();
  await expect(roles).toBeVisible();
  await page.getByRole("button", { name: "Rename", exact: true }).click();
  await dialog.getByLabel("Name").fill("Data Lab");
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(unit(page, "Data Lab")).toBeVisible();
  await expect(unit(page, "Databases Lab")).toHaveCount(0);

  await unit(page, "Data Lab").click();
  await page.getByRole("button", { name: "Delete", exact: true }).click();
  await dialog.getByRole("button", { name: "Delete" }).click();
  await expect(unit(page, "Data Lab")).toHaveCount(0);
});

test("a teacher who holds no unit role sees the unit of their course and cannot change it", async ({ page }) => {
  await signIn(page, "kowalska");
  await page.goto(`/course/settings?id=${await courseId(page, "Databases")}`);
  const unitSelect = page.getByLabel("Unit", { exact: true });
  await expect(unitSelect).toBeVisible();
  await expect(unitSelect).toBeDisabled();
  await expect(unitSelect.getByRole("option")).toHaveCount(1);
  await expect(unitSelect.getByRole("option", { name: "No unit" })).toHaveCount(0);
});

test("the owner restores an archived course from its settings", async ({ page }) => {
  await signIn(page, "teacher");
  await page.goto(`/course/settings?id=${await courseId(page, "Programming Fundamentals")}`);
  // An archived course reads as closed: the Settings tab stays for its owner, the edit controls do not.
  await expect(tab(page, "Settings")).toBeVisible();
  await expect(page.getByLabel("Course title")).toBeDisabled();
  await expect(page.getByRole("button", { name: "Save changes" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Delete course" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Transfer ownership" })).toBeVisible();
  await page.getByRole("button", { name: "Restore course" }).click();
  await expect(page.getByRole("button", { name: "Archive course" })).toBeVisible();
  await expect(page.getByLabel("Course title")).toBeEnabled();
  await expect(page.getByRole("button", { name: "Save changes" })).toBeVisible();
});

test("a student of an archived course has no Settings", async ({ page }) => {
  await signIn(page, "student");
  const id = await courseId(page, "Programming Fundamentals");
  await page.goto(`/course?id=${id}`);
  await expect(tab(page, "People")).toBeVisible();
  await expect(tab(page, "Settings")).toHaveCount(0);
  await page.goto(`/course/settings?id=${id}`);
  await expect(page).not.toHaveURL(/\/course\/settings/);
});
