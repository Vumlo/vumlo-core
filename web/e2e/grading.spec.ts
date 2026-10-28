import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD, type RoleName } from "./routes";
import { courseHref, gradeHref } from "../src/lib/routes";

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

type Mock = { course: string; item: string; rows: { id: string; name: string; former: boolean; status: string }[] };

/** The Merge sort assignment of the first course, with who is on it and what they handed in, read from the API. */
async function mergeSort(page: Page): Promise<Mock> {
  const course = await firstCourse(page);
  return page.evaluate(async (id) => {
    const j = async (p: string) => (await fetch(`/api/v1${p}`)).json();
    const book = await j(`/courses/${id}/gradebook`);
    const col = book.columns.find((c: { title: string }) => c.title === "Merge sort implementation");
    const subs = (await j(`/assignments/${col.assignment_id}/submissions`)).items;
    const rows = book.items.map((r: { user_id: string; name: string; surname: string; former?: boolean }) => ({
      id: r.user_id,
      name: `${r.name} ${r.surname}`,
      former: !!r.former,
      status: subs.find((s: { student_id: string }) => s.student_id === r.user_id)?.status ?? "none",
    }));
    return { course: id, item: col.assignment_id as string, rows };
  }, course);
}

const url = (page: Page) => new URL(page.url()).searchParams;

test("save and next moves past a graded student", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  await page.goto(gradeHref(m.course, m.item, m.rows[0]!.id));
  await page.getByLabel("Show", { exact: true }).selectOption({ label: "All" });
  const before = await page.locator("#student-h").innerText();
  await page.getByLabel("Points", { exact: true }).fill("20");
  await page.getByRole("button", { name: "Save and next" }).click();
  await expect(page.locator("#student-h")).not.toHaveText(before);
  await expect(page.locator("#student-h")).toHaveText(m.rows[1]!.name);
  expect(url(page).get("student")).toBe(m.rows[1]!.id);
});

test("next stays put at the end of the filtered list", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  const toGrade = m.rows.filter((r) => r.name === "Ivan Petrenko");
  await page.goto(gradeHref(m.course, m.item, toGrade[0]!.id));
  await expect(page.getByLabel("Show", { exact: true })).toHaveValue("grade");
  await expect(page.getByText("1 of 1", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Next student" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Previous student" })).toBeDisabled();
});

test("a student with nothing submitted can be graded", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  const nobody = m.rows.find((r) => r.status === "not_submitted" && !r.former)!;
  await page.goto(gradeHref(m.course, m.item, nobody.id));
  await expect(page.locator("#student-h")).toHaveText(nobody.name);
  await expect(page.getByRole("region", { name: "Submission" })).toContainText("Nothing was submitted.");
  await page.getByLabel("Points", { exact: true }).fill("12");
  await page.getByRole("button", { name: "Save draft" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Draft saved." })).toBeVisible();
});

test("a former student cannot be graded", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  const former = m.rows.find((r) => r.former)!;
  expect(former.name).toBe("Tomas Novak");
  await page.goto(gradeHref(m.course, m.item, former.id));
  await expect(page.locator("#student-h")).toHaveText("Tomas Novak");
  await expect(page.getByText("left the course")).toBeVisible();
  await expect(page.getByRole("button", { name: "Save draft" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Save and next" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Publish", exact: true })).toHaveCount(0);
});

test("points that are not a score are rejected without a request", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  const target = m.rows.find((r) => r.status === "not_submitted" && !r.former)!;
  await page.goto(gradeHref(m.course, m.item, target.id));
  const writes: string[] = [];
  page.on("request", (r) => { if (r.method() === "PUT" && /\/grades\//.test(r.url())) writes.push(r.url()); });
  const points = page.getByLabel("Points", { exact: true });
  for (const bad of ["abc", "999"]) {
    await points.fill(bad);
    await page.getByRole("button", { name: "Save draft" }).click();
    await expect(page.getByRole("alert").filter({ hasText: "Use a number from 0 to" })).toBeVisible();
  }
  expect(writes).toHaveLength(0);
  // The same observer sees a real save, so the zero above is not an observer that sees nothing.
  await points.fill("10");
  await page.getByRole("button", { name: "Save draft" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Draft saved." })).toBeVisible();
  expect(writes).toHaveLength(1);
});

test("leaving with unsaved points asks first", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  await page.goto(gradeHref(m.course, m.item, m.rows[0]!.id));
  await page.getByLabel("Show", { exact: true }).selectOption({ label: "All" });
  const here = await page.locator("#student-h").innerText();
  await page.getByLabel("Points", { exact: true }).fill("11");
  await page.getByRole("button", { name: "Next student" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Discard unsaved changes?");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator("#student-h")).toHaveText(here);
  await page.getByRole("button", { name: "Next student" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Discard" }).click();
  await expect(page.locator("#student-h")).not.toHaveText(here);
});

test("a saved draft becomes published", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  const target = m.rows.find((r) => r.status === "not_submitted" && !r.former)!;
  await page.goto(gradeHref(m.course, m.item, target.id));
  await page.getByLabel("Points", { exact: true }).fill("12");
  await page.getByRole("button", { name: "Save draft" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Draft saved." })).toBeVisible();
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByRole("status").filter({ hasText: `Published to ${target.name.split(" ")[0]}.` })).toBeVisible();
  await page.getByRole("link", { name: "Back to grades" }).click();
  await expect(page.getByRole("button", { name: new RegExp(`^${target.name}, Merge sort implementation, published 12`) })).toBeVisible();
});

test("the rail lists the filtered students and the picker lists everyone", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  await page.goto(gradeHref(m.course, m.item, m.rows[0]!.id));
  const rail = page.getByRole("navigation", { name: "Students" });
  await expect(rail.getByRole("button")).toHaveCount(1);
  await expect(rail.getByRole("button", { name: /Ivan Petrenko/ })).toContainText("To grade");
  await page.getByLabel("Show", { exact: true }).selectOption({ label: "Drafts" });
  await expect(rail.getByRole("button")).toHaveCount(4);
  await expect(page.getByLabel("Student", { exact: true }).locator("option")).toHaveCount(m.rows.length);
  // The student named in the address is outside the filter (Ivan is not a draft): the counter says so.
  await page.getByLabel("Student", { exact: true }).selectOption({ label: "Ivan Petrenko" });
  await expect(page.locator("#student-h")).toHaveText("Ivan Petrenko");
  await expect(page.getByText("– of 4", { exact: true })).toBeVisible();
  await rail.getByRole("button", { name: /Sofia Marchetti/ }).click();
  await expect(page.locator("#student-h")).toHaveText("Sofia Marchetti");
});

test("an empty list says so and still shows the student named in the address", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  const ivan = m.rows.find((r) => r.name === "Ivan Petrenko")!;
  await page.goto(gradeHref(m.course, m.item, ivan.id));
  await page.getByLabel("Points", { exact: true }).fill("15");
  await page.getByRole("button", { name: "Save draft" }).click();
  await expect(page.getByText("Nothing left to grade.")).toBeVisible();
  await expect(page.locator("#student-h")).toHaveText("Ivan Petrenko");
  await expect(page.getByRole("link", { name: "Back to the gradebook" })).toBeVisible();
});

test("a gradebook cell opens the workspace for that cell", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  await page.goto(courseHref(m.course, "grades"));
  await page.getByRole("button", { name: /^Omar Haddad, Merge sort implementation/ }).click();
  await expect(page).toHaveURL(/\/course\/grade\?/);
  expect(url(page).get("item")).toBe(m.item);
  expect(url(page).get("student")).toBe(m.rows.find((r) => r.name === "Omar Haddad")!.id);
  await expect(page.locator("#student-h")).toHaveText("Omar Haddad");
});

test("a former student's cell opens the workspace read-only", async ({ page }) => {
  await signIn(page, "teacher");
  const m = await mergeSort(page);
  await page.goto(courseHref(m.course, "grades"));
  await page.getByRole("button", { name: /^Tomas Novak, Merge sort implementation/ }).click();
  await expect(page.locator("#student-h")).toHaveText("Tomas Novak");
  await expect(page.getByRole("button", { name: "Save draft" })).toHaveCount(0);
});

async function openGradebook(page: Page) {
  await signIn(page, "teacher");
  const course = await firstCourse(page);
  await page.goto(courseHref(course, "grades"));
  await expect(page.getByRole("columnheader", { name: /Merge sort implementation/ })).toBeVisible();
}

const studentRows = (page: Page) => page.getByRole("row").filter({ has: page.getByRole("rowheader") });

test("the assignment filter shows one column", async ({ page }) => {
  await openGradebook(page);
  expect(await page.getByRole("columnheader").count()).toBeGreaterThan(2);
  await page.getByLabel("Assignment", { exact: true }).selectOption({ label: "Merge sort implementation" });
  await expect(page.getByRole("columnheader")).toHaveCount(2);
  await expect(page.getByRole("columnheader", { name: /Merge sort implementation/ })).toBeVisible();
});

test("the status filter keeps only matching rows", async ({ page }) => {
  await openGradebook(page);
  await page.getByLabel("Assignment", { exact: true }).selectOption({ label: "Merge sort implementation" });
  await page.getByLabel("Status", { exact: true }).selectOption({ label: "To grade" });
  await expect(studentRows(page)).toHaveCount(1);
  await expect(studentRows(page).getByRole("rowheader")).toContainText("Ivan Petrenko");
  await expect(studentRows(page).getByRole("button")).toHaveAccessibleName(/to grade/i);
});

test("filters that match nobody say so", async ({ page }) => {
  await openGradebook(page);
  await page.getByLabel("Assignment", { exact: true }).selectOption({ label: "Sorting quiz" });
  await page.getByLabel("Status", { exact: true }).selectOption({ label: "Published" });
  await expect(page.getByText("No students match.")).toBeVisible();
  await expect(studentRows(page)).toHaveCount(0);
});

test("late only hides rows that handed nothing in late", async ({ page }) => {
  await openGradebook(page);
  const before = await studentRows(page).count();
  expect(before).toBeGreaterThan(1);
  await page.getByLabel("Late only").check();
  // The mock holds no late attempt, so nobody is left; the unit tests of matchesStatus cover the late rule itself.
  await expect(page.getByText("No students match.")).toBeVisible();
  await page.getByLabel("Late only").uncheck();
  await expect(studentRows(page)).toHaveCount(before);
});
