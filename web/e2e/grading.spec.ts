import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD, type RoleName } from "./routes";
import { gradeHref } from "../src/lib/routes";

async function signIn(page: Page, role: RoleName) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts[role]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  await expect(page.locator("header").first()).toBeVisible();
}

/** The first course of the signed-in person, from the API. */
async function firstCourse(page: Page): Promise<string> {
  return page.evaluate(async () => (await (await fetch("/api/v1/me/courses?limit=100")).json()).items[0].id as string);
}

test("the workspace shows the submission of the student it names", async ({ page }) => {
  await signIn(page, "teacher");
  await page.goto("/grading");
  const first = page.getByRole("row").filter({ has: page.getByRole("link", { name: "Grade", exact: true }) }).first();
  await expect(first).toBeVisible();
  const student = (await first.locator("span[role=cell]").first().locator("span").last().innerText()).trim();
  await first.getByRole("link", { name: "Grade", exact: true }).click();
  await expect(page).toHaveURL(/\/course\/grade\?/);
  await expect(page.locator("#student-h")).toHaveText(student);
  const region = page.getByRole("region", { name: "Submission" });
  await expect(region).toContainText("Submitted");
  await expect(region).not.toContainText("Nothing was submitted.");
});

test("a quiz or form attempt is drawn as questions", async ({ page }) => {
  await signIn(page, "teacher");
  const course = await firstCourse(page);
  // The mock holds no quiz attempt, so the first quiz or form assignment that somebody submitted stands for both.
  const target = await page.evaluate(async (id) => {
    const j = async (p: string) => (await fetch(`/api/v1${p}`)).json();
    const tree = await j(`/courses/${id}/content`);
    const items = tree.modules.flatMap((m: { items: { id: string; kind: string }[]; submodules: { items: { id: string; kind: string }[] }[] }) => [...m.items, ...m.submodules.flatMap((s) => s.items)]);
    for (const it of items.filter((i: { kind: string }) => i.kind === "assignment")) {
      const type = (await j(`/items/${it.id}`)).assignment?.type;
      if (type !== "quiz" && type !== "form") continue;
      const sub = (await j(`/assignments/${it.id}/submissions`)).items.find((s: { status: string }) => s.status === "submitted");
      if (sub) return { item: it.id as string, student: sub.student_id as string };
    }
    return null;
  }, course);
  expect(target).not.toBeNull();
  await page.goto(gradeHref(course, target!.item, target!.student));
  const region = page.getByRole("region", { name: "Submission" });
  await expect(region.getByRole("radio").or(region.getByRole("checkbox")).or(region.getByRole("textbox")).first()).toBeVisible();
  await expect(region.getByRole("heading", { level: 2 }).first()).toBeVisible();
});

test("an unknown assignment is not found", async ({ page }) => {
  await signIn(page, "teacher");
  const course = await firstCourse(page);
  await page.goto(gradeHref(course, "00000000-0000-0000-0000-000000000000"));
  await expect(page.getByRole("heading", { name: "We can't find that page" })).toBeVisible();
});

test("an address without an assignment is not found", async ({ page }) => {
  await signIn(page, "teacher");
  const course = await firstCourse(page);
  await page.goto(`/course/grade?id=${course}`);
  await expect(page.getByRole("heading", { name: "We can't find that page" })).toBeVisible();
});
