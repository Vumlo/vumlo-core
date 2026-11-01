import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD } from "./routes";

async function signInAsTeacher(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.teacher);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
}

test("every tab says where you are first, then the course, then the app", async ({ page }) => {
  await page.goto("/login");
  await expect(page).toHaveTitle("Sign in · Vumlo");
  await signInAsTeacher(page);
  // Moving by links, not by address: the title follows the client-side navigation too.
  await expect(page).toHaveTitle("Home · Vumlo");
  await page.getByRole("navigation", { name: "Main" }).first().getByRole("link", { name: "Courses" }).click();
  await expect(page).toHaveTitle("Courses · Vumlo");
  await page.getByRole("navigation", { name: "Main" }).first().getByRole("link", { name: "Calendar" }).click();
  await expect(page).toHaveTitle("Calendar · Vumlo");

  const id = await page.evaluate(async () => (await (await fetch("/api/v1/me/courses?limit=100")).json()).items[0].id as string);
  await page.goto(`/course?id=${id}`);
  await expect(page).toHaveTitle("Algorithms and Data Structures · Vumlo");
  await page.goto(`/course/people?id=${id}`);
  await expect(page).toHaveTitle("People · Algorithms and Data Structures · Vumlo");
  await page.goto(`/course/grades?id=${id}`);
  await expect(page).toHaveTitle("Gradebook · Algorithms and Data Structures · Vumlo");
});

test("an item names itself, and its editor says so", async ({ page }) => {
  await signInAsTeacher(page);
  const found = await page.evaluate(async () => {
    const courses = (await (await fetch("/api/v1/me/courses?limit=100")).json()) as { items: { id: string; title: string }[] };
    for (const c of courses.items) {
      const tree = (await (await fetch(`/api/v1/courses/${c.id}/content`)).json()) as { modules: { items: { id: string; title: string }[] }[] };
      const hit = tree.modules.flatMap((m) => m.items).find((i) => i.title === "Lab report (files)");
      if (hit) return { course: c.id, courseTitle: c.title, item: hit.id };
    }
    throw new Error("the seeded file assignment is missing");
  });
  await page.goto(`/course/item?id=${found.course}&item=${found.item}`);
  await expect(page).toHaveTitle(`Lab report (files) · ${found.courseTitle} · Vumlo`);
  await page.goto(`/course/item/edit?id=${found.course}&item=${found.item}`);
  await expect(page).toHaveTitle(`Edit: Lab report (files) · ${found.courseTitle} · Vumlo`);
});

test("the favicon and the logo are Vumlo's", async ({ page }) => {
  await page.goto("/login");
  const icons = await page.evaluate(() => [...document.querySelectorAll("link[rel~=icon],link[rel=apple-touch-icon]")].map((l) => l.getAttribute("href")));
  expect(icons).toEqual(["/brand/favicon.ico", "/brand/icon-192.png", "/brand/apple-touch-icon.png"]);
  for (const href of icons) expect((await page.request.get(href as string)).ok(), `${href} is served`).toBe(true);
  const logo = page.locator("img[src='/vumlo-mark.png']").first();
  await expect(logo).toBeVisible();
  expect(await logo.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);
});
