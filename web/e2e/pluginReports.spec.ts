import { expect, test, type ConsoleMessage, type Page } from "@playwright/test";
import { courseHref, gradeHref, itemHref } from "../src/lib/routes";
import { accounts, PASSWORD, store, type RoleName } from "./routes";

// The "Plugin reports" panel (report display convention v1) on the three staff views, against the mock reports
// of src/mocks/handlers/pluginReports.ts.

async function signIn(page: Page, role: RoleName) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts[role]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
}

function watch(page: Page) {
  const problems: string[] = [];
  page.on("console", (m: ConsoleMessage) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  return problems;
}

/** The first course, the Merge sort assignment and a student who handed it in, from the API like the other specs. */
async function find(page: Page) {
  return page.evaluate(async () => {
    const j = async (p: string) => (await fetch(`/api/v1${p}`)).json();
    const course = ((await j("/me/courses?limit=100")).items as { id: string }[])[0]!.id;
    const book = await j(`/courses/${course}/gradebook`);
    const col = book.columns.find((c: { title: string }) => c.title === "Merge sort implementation");
    const subs = (await j(`/assignments/${col.assignment_id}/submissions`)).items as { student_id: string; latest: unknown }[];
    const student = subs.find((s) => s.latest)!.student_id;
    return { course, item: col.assignment_id as string, student };
  });
}

const panel = (page: Page) => page.getByRole("region", { name: "Plugin reports" });

for (const theme of ["light", "dark"] as const) {
  test(`teacher: the course page lists the reports with a table, facts and links (${theme})`, async ({ page }) => {
    await store(page, "vumlo.theme", theme);
    const problems = watch(page);
    await signIn(page, "teacher");
    const { course } = await find(page);
    await page.goto(courseHref(course));

    const reports = panel(page);
    await expect(reports.getByRole("heading", { level: 2, name: "Plugin reports" })).toBeVisible();
    const gradebook = reports.getByRole("article", { name: "Gradebook export" });
    await expect(gradebook.getByText("OK", { exact: true })).toBeVisible();
    await expect(gradebook.getByText("The gradebook is ready for download.")).toBeVisible();
    // The links are plain anchors on the plugin's routes.
    const csv = gradebook.getByRole("link", { name: "Download CSV" });
    await expect(csv).toHaveAttribute("href", new RegExp(`^/api/v1/plugins/gradebook-export/export/${course}/csv$`));
    await expect(gradebook.getByRole("link", { name: "Download XLSX" })).toBeVisible();

    // A long table: accessible markup, 200 rows, numbers in the viewer's language, a note about the rest.
    const risk = reports.getByRole("article", { name: "At-risk students" });
    const table = risk.getByRole("table", { name: "At-risk students" });
    await expect(table.getByRole("columnheader")).toHaveText(["Student", "Average, %", "Risk"]);
    await expect(table.getByRole("row")).toHaveCount(201); // the header and 200 rows
    await expect(table.getByRole("rowheader").first()).toHaveText("Student 1");
    await expect(table.getByRole("cell").first()).toHaveText("1,000");
    await expect(risk.getByText("Showing the first 200 of 230 rows.")).toBeVisible();
    // It scrolls on its own, and the keyboard can reach it.
    const region = risk.getByRole("region", { name: "At-risk students, table" });
    await region.focus();
    await expect(region).toBeFocused();
    expect(problems).toEqual([]);
  });
}

test("the numbers follow the language of the viewer", async ({ page }) => {
  // The sign-in form is in English; the language is switched after it, as a person would in the settings.
  await signIn(page, "teacher");
  const { course } = await find(page);
  await page.evaluate(() => localStorage.setItem("vumlo.language", "uk"));
  await page.goto(courseHref(course));
  const reports = page.getByRole("region", { name: "Звіти плагінів" });
  const table = reports.getByRole("table", { name: "At-risk students" });
  await expect(table.getByRole("cell").first()).toHaveText(/^1\s000$/);
  await expect(reports.getByText("Показано перші 200 з 230 рядків.")).toBeVisible();
  await expect(reports.getByText("Інформація", { exact: true })).toBeVisible();
});

test("teacher: the assignment page shows the reports, drops foreign links, and falls back for a broken display", async ({ page }) => {
  const problems = watch(page);
  await signIn(page, "teacher");
  const { course, item } = await find(page);
  await page.goto(itemHref(course, item));

  const reports = panel(page);
  const anomalies = reports.getByRole("article", { name: "Unusual grades" });
  await expect(anomalies.getByText("Alert", { exact: true })).toBeVisible();
  await expect(anomalies.getByRole("table").getByRole("row")).toHaveCount(3);
  await expect(anomalies.getByRole("definition").first()).toBeVisible();
  // Only the plugin's own route became a link.
  await expect(anomalies.getByRole("link")).toHaveCount(1);
  await expect(anomalies.getByRole("link", { name: "Download the list" })).toHaveAttribute("href", "/api/v1/plugins/grade-anomaly-report/export/anomalies.csv");

  // The display of this one does not follow the convention: its summary, the plugin and key as the title, and the raw data folded away.
  const broken = reports.getByRole("article", { name: "quiz-item-analysis · broken" });
  await expect(broken.getByText("Analysis is not ready.")).toBeVisible();
  await expect(broken.getByRole("table")).toHaveCount(0);
  await expect(broken.getByText("Alert", { exact: true })).toHaveCount(0);
  const raw = broken.locator("details");
  await expect(raw).not.toHaveAttribute("open", "");
  await broken.getByText("Raw data").click();
  await expect(raw.locator("pre")).toContainText('"columns"');
  expect(problems).toEqual([]);
});

test("teacher: the submission in the grading workspace shows the reports; text is text", async ({ page }) => {
  const problems = watch(page);
  await signIn(page, "teacher");
  const { course, item, student } = await find(page);
  await page.goto(gradeHref(course, item, student));

  const reports = panel(page);
  const metrics = reports.getByRole("article", { name: "Length of the text" });
  await expect(metrics.getByText("Warning", { exact: true })).toBeVisible();
  await expect(metrics.getByText("Words", { exact: true })).toBeVisible();
  await expect(metrics.getByText("2,845")).toBeVisible();

  // Markup and markdown in a summary are shown as typed, never interpreted.
  const legacy = reports.getByRole("article", { name: "legacy-check · legacy" });
  await expect(legacy.getByText("Nothing suspicious <b>found</b> & **checked**.")).toBeVisible();
  await expect(legacy.locator("b, strong")).toHaveCount(0);
  expect(problems).toEqual([]);
});

test("a student never sees the panel and the reports are not asked for", async ({ page }) => {
  const asked: string[] = [];
  page.on("request", (r) => {
    if (r.url().includes("plugin-reports")) asked.push(r.url());
  });
  await signIn(page, "student");
  const { course, item } = await page.evaluate(async () => {
    const j = async (p: string) => (await fetch(`/api/v1${p}`)).json();
    const course = ((await j("/me/courses?limit=100")).items as { id: string }[])[0]!.id;
    const tree = await j(`/courses/${course}/content`);
    const items = tree.modules.flatMap((m: { items: { id: string; kind: string }[] }) => m.items);
    return { course, item: items.find((i: { kind: string }) => i.kind === "assignment").id as string };
  });
  await page.goto(courseHref(course));
  await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
  await page.goto(itemHref(course, item));
  await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
  await expect(panel(page)).toHaveCount(0);
  expect(asked).toEqual([]);
  // And the server refuses the student if the screen is skipped.
  const status = await page.evaluate(async (id) => (await fetch(`/api/v1/assignments/${id}/plugin-reports`)).status, item);
  expect(status).toBe(403);
});
