import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { accounts, PASSWORD } from "./routes";
import { gradeHref } from "../src/lib/routes";

// Uploading and opening files (spec 9.1). No screen uses them yet, so the mock build puts the two functions on
// `window.__vumloFiles` (src/mocks/StartWorker.tsx; a real build has no such object).
declare global {
  interface Window {
    __vumloFiles: {
      uploadFile(file: File, target: { item: string } | { assignment: string }): Promise<{ id: string; name: string; mime: string; size: number }>;
      openFile(ref: { id: string; name: string; mime: string; size: number }): Promise<void>;
      setMaxSize(bytes: number): void;
      fileUrl(id: string): string;
      readFileText(id: string, maxBytes: number): Promise<string>;
    };
  }
}

async function signIn(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.teacher);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
  await page.waitForFunction(() => "__vumloFiles" in window);
}

/** The id of some item of the teacher's first course. */
async function anItem(page: Page): Promise<string> {
  return page.evaluate(async () => {
    const courses = (await (await fetch("/api/v1/me/courses?limit=100")).json()) as { items: { id: string }[] };
    const tree = (await (await fetch(`/api/v1/courses/${courses.items[0]!.id}/content`)).json()) as { modules: { items: { id: string }[] }[] };
    return tree.modules.flatMap((m) => m.items)[0]!.id;
  });
}

type Sent = { url: URL; type: string | undefined };

/** What the page sent to POST /files, kept as it happens. */
function watchUploads(page: Page): Sent[] {
  const sent: Sent[] = [];
  page.on("request", (r) => {
    if (r.method() === "POST" && new URL(r.url()).pathname === "/api/v1/files") {
      sent.push({ url: new URL(r.url()), type: r.headers()["content-type"] });
    }
  });
  return sent;
}

const upload = (page: Page, item: string, name: string, type: string, size: number) =>
  page.evaluate(
    ([id, n, t, len]) => window.__vumloFiles.uploadFile(new File([new Uint8Array(len as number).fill(7)], n as string, { type: t as string }), { item: id as string }),
    [item, name, type, size],
  );

test("upload_posts_raw_bytes_with_the_file_type", async ({ page }) => {
  await signIn(page);
  const item = await anItem(page);
  const sent = watchUploads(page);
  const ref = await upload(page, item, "Звіт з фізики & нотатки.pdf", "application/pdf", 1234);
  expect(sent).toHaveLength(1);
  expect(sent[0]!.type).toBe("application/pdf");
  expect(sent[0]!.url.searchParams.get("for")).toBe(`item:${item}`);
  expect(sent[0]!.url.searchParams.get("name")).toBe("Звіт з фізики & нотатки.pdf");
  // The name travels percent-encoded, not as a raw `&` that would end the parameter.
  expect(sent[0]!.url.search).toContain(`name=${encodeURIComponent("Звіт з фізики & нотатки.pdf")}`);
  // The mock measures the body it received, so the size it answers with is the number of raw bytes on the wire.
  expect(ref).toMatchObject({ name: "Звіт з фізики & нотатки.pdf", mime: "application/pdf", size: 1234 });
});

test("open_pdf_opens_a_new_tab", async ({ page }) => {
  await signIn(page);
  const ref = await upload(page, await anItem(page), "syllabus.pdf", "application/pdf", 50);
  // The headless browser has no PDF viewer, so the new tab's address stays empty; what is checked is the call it was opened with.
  await page.evaluate(() => {
    const open = window.open.bind(window);
    const calls: unknown[][] = [];
    Object.assign(window, { __opened: calls });
    window.open = (...args: Parameters<typeof window.open>) => (calls.push(args), open(...args));
  });
  await Promise.all([page.waitForEvent("popup"), page.evaluate((r) => window.__vumloFiles.openFile(r), ref)]);
  expect(await page.evaluate(() => (window as unknown as { __opened: unknown[] }).__opened)).toEqual([[`/api/v1/files/${ref.id}`, "_blank", "noopener"]]);
});

test("open_docx_triggers_a_download_with_its_name", async ({ page }) => {
  await signIn(page);
  const docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
  const ref = await upload(page, await anItem(page), "Lab report 3.docx", docx, 50);
  const [download] = await Promise.all([page.waitForEvent("download"), page.evaluate((r) => window.__vumloFiles.openFile(r), ref)]);
  expect(download.suggestedFilename()).toBe("Lab report 3.docx");
});

test("open_file_in_desktop_calls_the_open_file_command", async ({ page }) => {
  await signIn(page);
  const ref = await upload(page, await anItem(page), "notes.docx", "application/octet-stream", 50);
  await page.evaluate(() => {
    const calls: { cmd: string; args: unknown }[] = [];
    Object.assign(window, {
      __calls: calls,
      __TAURI_INTERNALS__: {
        async invoke(cmd: string, args: unknown) {
          calls.push({ cmd, args });
          return "saved_only";
        },
      },
    });
  });
  await page.evaluate((r) => window.__vumloFiles.openFile(r), ref);
  const calls = await page.evaluate(() => (window as unknown as { __calls: unknown[] }).__calls);
  expect(calls).toEqual([{ cmd: "open_file", args: { id: ref.id } }]);
  await expect(page.getByRole("status").filter({ hasText: "notes.docx saved to Downloads" })).toBeVisible();
});

test("upload_over_the_limit_shows_the_too_large_message", async ({ page }) => {
  await signIn(page);
  const item = await anItem(page);
  await page.evaluate(() => window.__vumloFiles.setMaxSize(10));
  const sent = watchUploads(page);
  const message = await page.evaluate(
    async (id) => {
      try {
        await window.__vumloFiles.uploadFile(new File([new Uint8Array(100)], "big.pdf", { type: "application/pdf" }), { item: id });
        return "uploaded";
      } catch (e) {
        return (e as Error).message;
      }
    },
    item,
  );
  expect(sent).toHaveLength(1);
  expect(message).toBe("This file is too large.");
});

// The desktop window loads files through a custom protocol; Tauri alone knows its address form (it differs between
// Windows and the other systems), so the stand-in's `convertFileSrc` has a form of its own that no literal matches.
const ID = "0198f3a0-7b1c-7d2e-8f3a-5b6c7d8e9f01";
const fakeConvert = (path: string, protocol: string) => `http://${protocol}.localhost/${path}`;
async function asDesktop(page: Page) {
  await page.evaluate((src) => {
    const convert = new Function("return " + src)() as (p: string, q: string) => string;
    Object.assign(window, { __TAURI_INTERNALS__: { convertFileSrc: convert, invoke: async () => null } });
  }, fakeConvert.toString());
}

test("desktop_file_url_uses_the_tauri_custom_protocol_form", async ({ page }) => {
  await signIn(page);
  await asDesktop(page);
  expect(await page.evaluate((id) => window.__vumloFiles.fileUrl(id), ID)).toBe(`http://vumlo-file.localhost/${ID}`);
});

test("desktop_read_file_text_fetches_that_url", async ({ page }) => {
  await signIn(page);
  // The mock's service worker sees every request of the page, so the protocol's answer is faked at `fetch` itself.
  await page.evaluate(() => {
    const requested: { url: string; range: string | null }[] = [];
    Object.assign(window, { __requested: requested });
    window.fetch = async (input, init) => {
      requested.push({ url: String(input), range: new Headers(init?.headers).get("range") });
      return new Response("0123456789", { headers: { "Content-Type": "text/plain" } });
    };
  });
  await asDesktop(page);
  const text = await page.evaluate((id) => window.__vumloFiles.readFileText(id, 4), ID);
  expect(text).toBe("0123");
  expect(await page.evaluate(() => (window as unknown as { __requested: unknown }).__requested)).toEqual([{ url: `http://vumlo-file.localhost/${ID}`, range: "bytes=0-3" }]);
});

test("desktop_upload_sends_the_bytes_as_the_body_and_the_rest_as_headers", async ({ page }) => {
  await signIn(page);
  const item = await anItem(page);
  await page.evaluate(() => {
    const calls: { cmd: string; args: unknown; options: unknown }[] = [];
    Object.assign(window, {
      __calls: calls,
      __TAURI_INTERNALS__: {
        async invoke(cmd: string, args: unknown, options: unknown) {
          calls.push({ cmd, args, options });
          if ((args as Uint8Array).length === 3) throw "413: too big";
          return { id: "0198f3a0-7b1c-7d2e-8f3a-5b6c7d8e9f01", name: "a & b.pdf", mime: "application/pdf", size: 5 };
        },
      },
    });
  });
  const ref = await page.evaluate((id) => window.__vumloFiles.uploadFile(new File([new Uint8Array(5).fill(7)], "a & b.pdf", { type: "application/pdf" }), { item: id }), item);
  expect(ref).toMatchObject({ name: "a & b.pdf", size: 5 });
  const calls = await page.evaluate(() => (window as unknown as { __calls: { cmd: string; args: number[]; options: unknown }[] }).__calls.map((c) => ({ ...c, args: Array.from(c.args as unknown as Uint8Array) })));
  expect(calls).toEqual([
    {
      cmd: "upload_file",
      args: [7, 7, 7, 7, 7],
      options: { headers: { "x-vumlo-target": `item:${item}`, "x-vumlo-name": encodeURIComponent("a & b.pdf"), "x-vumlo-mime": "application/pdf" } },
    },
  ]);
  // The command's "<status>: <detail>" refusal becomes an ApiError of that status.
  const status = await page.evaluate(
    (id) => window.__vumloFiles.uploadFile(new File([new Uint8Array(3)], "c.pdf", { type: "application/pdf" }), { item: id }).then(() => 0, (e: { status: number }) => e.status),
    item,
  );
  expect(status).toBe(413);
});

// ---- the `file` item kind: lesson material (spec 7, 9.3) ----

async function signInAs(page: Page, role: "student" | "teacher") {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts[role]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
}

const SYLLABUS = "Course syllabus";
const pdf = (name: string) => ({ name, mimeType: "application/pdf", buffer: Buffer.from("%PDF-1.4\n% a file for the specs\n%%EOF\n") });

/** The id of the course and of the seeded file item (titled "Course syllabus"), from the API. */
async function syllabus(page: Page): Promise<{ course: string; item: string }> {
  return page.evaluate(async (title) => {
    const courses = (await (await fetch("/api/v1/me/courses?limit=100")).json()) as { items: { id: string }[] };
    for (const c of courses.items) {
      const tree = (await (await fetch(`/api/v1/courses/${c.id}/content`)).json()) as { modules: { items: { id: string; title: string }[] }[] };
      const hit = tree.modules.flatMap((m) => m.items).find((i) => i.title === title);
      if (hit) return { course: c.id, item: hit.id };
    }
    throw new Error("the seeded file item is missing");
  }, SYLLABUS);
}

/** The bodies of the requests the page sends with `method` to a path that matches `path`. */
function watchBodies(page: Page, method: string, path: RegExp): Record<string, unknown>[] {
  const bodies: Record<string, unknown>[] = [];
  page.on("request", (r) => {
    if (r.method() === method && path.test(new URL(r.url()).pathname)) bodies.push(JSON.parse(r.postData() ?? "{}") as Record<string, unknown>);
  });
  return bodies;
}

async function newFileItem(page: Page, title: string) {
  await page.goto(`/course?id=${(await syllabus(page)).course}`);
  await page.getByRole("button", { name: "File", exact: true }).first().click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Title").fill(title);
  await dialog.getByRole("button", { name: "Create and edit" }).click();
  await page.waitForURL(/\/course\/item\/edit/);
}

test("teacher_adds_a_pdf_as_lesson_material", async ({ page }) => {
  await signInAs(page, "teacher");
  await page.waitForFunction(() => "__vumloFiles" in window);
  const created = watchBodies(page, "POST", /\/modules\/[^/]+\/items$/);
  const sent = watchUploads(page);
  const patches = watchBodies(page, "PATCH", /\/items\/[^/]+$/);
  await newFileItem(page, "Lab handout");
  expect(created).toEqual([expect.objectContaining({ kind: "file", title: "Lab handout" })]);
  // A file item cannot be published before it has a file; the page says so and does not offer the click.
  const publish = page.getByRole("button", { name: "Publish", exact: true });
  await expect(publish).toBeDisabled();
  await expect(page.getByText("Add a file before you publish.")).toBeVisible();
  await page.locator('input[type="file"]').setInputFiles(pdf("handout.pdf"));
  await expect(page.getByText("handout.pdf")).toBeVisible();
  await expect(publish).toBeEnabled();
  // The upload goes to the item that exists, then the body names it.
  const item = new URL(page.url()).searchParams.get("item")!;
  expect(sent).toHaveLength(1);
  expect(sent[0]!.url.searchParams.get("for")).toBe(`item:${item}`);
  await publish.click();
  await expect(page.locator("[data-tone]", { hasText: "Published" }).first()).toBeVisible();
  const last = patches.at(-1)!;
  expect((last.body as { file_id?: string }).file_id).toMatch(/^[0-9a-f-]+$/);
  const stored = await page.evaluate(async (id) => (await (await fetch(`/api/v1/items/${id}`)).json()) as { status: string; body: { file_id?: string } }, item);
  expect(stored.status).toBe("published");
  expect(stored.body.file_id).toBe((last.body as { file_id: string }).file_id);
});

test("student_sees_the_file_row_and_opens_it", async ({ page }) => {
  await signInAs(page, "student");
  await page.goto(`/course?id=${(await syllabus(page)).course}`);
  // The outline on the left names it too; the row is the one in the list.
  const row = page.locator("a[class*=rowLecture]").filter({ hasText: SYLLABUS });
  await expect(row).toBeVisible();
  // The file has a tile of its own, not the lecture's or the assignment's.
  await expect(row.locator(".tile-file")).toBeVisible();
  await expect(row).toContainText("File");
  await row.click();
  await expect(page.getByRole("heading", { name: SYLLABUS, level: 1 })).toBeVisible();
  await page.evaluate(() => {
    const open = window.open.bind(window);
    const calls: unknown[][] = [];
    Object.assign(window, { __opened: calls });
    window.open = (...args: Parameters<typeof window.open>) => (calls.push(args), open(...args));
  });
  await Promise.all([page.waitForEvent("popup"), page.getByRole("button", { name: "Open", exact: true }).click()]);
  const [url, target] = (await page.evaluate(() => (window as unknown as { __opened: string[][] }).__opened))[0]!;
  expect(url).toMatch(/^\/api\/v1\/files\/[0-9a-f-]+$/);
  expect(target).toBe("_blank");
});

test("file_item_page_shows_description_and_open_card", async ({ page }) => {
  await signInAs(page, "student");
  const { course, item } = await syllabus(page);
  await page.goto(`/course/item?id=${course}&item=${item}`);
  await expect(page.getByText("Read this before the first lab.")).toBeVisible();
  const card = page.getByRole("group", { name: "File" });
  await expect(card).toContainText("syllabus.pdf");
  await expect(card).toContainText(/\d+ (B|KB)/);
  await expect(card).toContainText("Opens in a new tab");
  await expect(card.getByRole("button", { name: "Open", exact: true })).toBeVisible();
  // No editing for a student.
  await expect(page.getByRole("link", { name: "Edit" })).toHaveCount(0);
});

test("replacing_the_file_updates_the_card", async ({ page }) => {
  await signInAs(page, "teacher");
  const { course, item } = await syllabus(page);
  await page.goto(`/course/item/edit?id=${course}&item=${item}`);
  const card = page.getByRole("group", { name: "File" });
  await expect(card).toContainText("syllabus.pdf");
  await page.locator('input[type="file"]').setInputFiles(pdf("syllabus-v2.pdf"));
  await expect(card).toContainText("syllabus-v2.pdf");
  await expect(card).not.toContainText("syllabus.pdf");
  // The page of the item shows the new file once the change is saved.
  await expect(page.getByRole("status").filter({ hasText: /^Saved/ })).toBeVisible();
  await page.getByRole("link", { name: "Preview as student" }).click();
  await expect(page.getByRole("group", { name: "File" })).toContainText("syllabus-v2.pdf");
});

test("publishing_a_file_item_without_a_file_shows_the_server_message", async ({ page }) => {
  await signInAs(page, "teacher");
  await newFileItem(page, "Empty handout");
  await page.goBack();
  await page.getByRole("button", { name: "More actions for Empty handout" }).click();
  await page.getByRole("menuitem", { name: "Publish" }).click();
  await expect(page.getByRole("alert").filter({ hasText: "a file item needs a file before it is published" })).toBeVisible();
});

test("removing_the_file_omits_file_id_from_the_patch", async ({ page }) => {
  await signInAs(page, "teacher");
  const { course, item } = await syllabus(page);
  await page.goto(`/course/item/edit?id=${course}&item=${item}`);
  await expect(page.getByRole("group", { name: "File" })).toContainText("syllabus.pdf");
  const patches = watchBodies(page, "PATCH", /\/items\/[^/]+$/);
  await page.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByRole("group", { name: "File" })).toHaveCount(0);
  await expect.poll(() => patches.length).toBeGreaterThan(0);
  const body = patches.at(-1)!.body as Record<string, unknown>;
  expect("file_id" in body).toBe(false);
  // What the teacher wrote about the file stays.
  expect(JSON.stringify(body.description)).toContain("Read this before the first lab.");
});

test("a_save_naming_a_swept_upload_asks_to_upload_the_file_again", async ({ page }) => {
  await signInAs(page, "teacher");
  await newFileItem(page, "Swept handout");
  // The server sweeps an upload nobody named for 24 hours and then refuses a body that names it.
  await page.evaluate(() => {
    const real = window.fetch.bind(window);
    window.fetch = async (input, init) => {
      if (init?.method === "PATCH" && /\/api\/v1\/items\//.test(String(input))) {
        const problem = { title: "Unprocessable", status: 422, errors: [{ field: "body.file_id", message: "the file cannot be used here" }] };
        return new Response(JSON.stringify(problem), { status: 422, headers: { "Content-Type": "application/problem+json" } });
      }
      return real(input, init);
    };
  });
  await page.locator('input[type="file"]').setInputFiles(pdf("old.pdf"));
  await expect(page.getByRole("alert").filter({ hasText: "Upload it again." })).toBeVisible();
  // The form stays usable: another file can be chosen.
  await expect(page.getByRole("button", { name: "Replace", exact: true })).toBeEnabled();
});

// ---- the `file` assignment type: a student hands in files (spec 8, 9.3) ----

const REPORT = "Lab report (files)";
const doc = (name: string, size = 64) => ({ name, mimeType: name.endsWith(".pdf") ? "application/pdf" : "application/octet-stream", buffer: Buffer.alloc(size, 7) });

/** The id of the course and of the seeded file assignment (max 3 files; .pdf, .docx and .png), from the API. */
async function report(page: Page): Promise<{ course: string; item: string }> {
  return page.evaluate(async (title) => {
    const courses = (await (await fetch("/api/v1/me/courses?limit=100")).json()) as { items: { id: string }[] };
    for (const c of courses.items) {
      const tree = (await (await fetch(`/api/v1/courses/${c.id}/content`)).json()) as { modules: { items: { id: string; title: string }[] }[] };
      const hit = tree.modules.flatMap((m) => m.items).find((i) => i.title === title);
      if (hit) return { course: c.id, item: hit.id };
    }
    throw new Error("the seeded file assignment is missing");
  }, REPORT);
}

async function openReport(page: Page, role: "student" | "teacher" = "student") {
  await signInAs(page, role);
  await page.waitForFunction(() => "__vumloFiles" in window);
  const ids = await report(page);
  await page.goto(`/course/item?id=${ids.course}&item=${ids.item}`);
  await expect(page.getByRole("heading", { name: REPORT, level: 1 })).toBeVisible();
  return ids;
}

const picker = (page: Page) => page.locator('input[type="file"]');
const submitButton = (page: Page) => page.getByRole("button", { name: "Submit files", exact: true });
const filesList = (page: Page) => page.getByRole("list", { name: "Your files" });

test("student_drops_two_files_and_submits", async ({ page }) => {
  await openReport(page);
  const sent = watchUploads(page);
  const saved = watchBodies(page, "PUT", /\/submission\/draft$/);
  const submitted: string[] = [];
  page.on("request", (r) => r.method() === "POST" && /\/submission\/submit$/.test(r.url()) && submitted.push(r.url()));
  // The accept attribute comes from the assignment's config.
  await expect(picker(page)).toHaveAttribute("accept", ".pdf,.docx,.png");
  await expect(picker(page)).toHaveAttribute("multiple", "");
  // A drop carries the same files as the picker; the specs use the picker (headless drag and drop is not reliable).
  await picker(page).setInputFiles([doc("report.pdf"), doc("data.docx", 2048)]);
  await expect(filesList(page).getByRole("listitem")).toHaveCount(2);
  await expect(filesList(page)).toContainText("report.pdf");
  await expect(filesList(page)).toContainText("data.docx");
  await expect(filesList(page)).toContainText("2 KB");
  expect(sent.map((s) => s.url.searchParams.get("name")).sort()).toEqual(["data.docx", "report.pdf"]);
  const assignment = new URL(page.url()).searchParams.get("item")!;
  expect(sent.every((s) => s.url.searchParams.get("for") === `assignment:${assignment}`)).toBe(true);
  await expect.poll(() => saved.length).toBeGreaterThan(0);
  expect((saved.at(-1)!.content as { files: string[] }).files).toHaveLength(2);
  await expect(submitButton(page)).toBeEnabled();
  await submitButton(page).click();
  await expect(page.getByText(/^Submitted, attempt 1/).first()).toBeVisible();
  expect(submitted).toHaveLength(1);
});

test("student_removes_a_file_before_submitting", async ({ page }) => {
  await openReport(page);
  const saved = watchBodies(page, "PUT", /\/submission\/draft$/);
  await picker(page).setInputFiles([doc("a.pdf"), doc("b.pdf")]);
  await expect(filesList(page).getByRole("listitem")).toHaveCount(2);
  await expect.poll(() => (saved.at(-1)?.content as { files: string[] } | undefined)?.files.length).toBe(2);
  await page.getByRole("button", { name: "Remove a.pdf" }).click();
  await expect(filesList(page).getByRole("listitem")).toHaveCount(1);
  await expect(filesList(page)).not.toContainText("a.pdf");
  await expect.poll(() => (saved.at(-1)?.content as { files: string[] }).files.length).toBe(1);
  // The kept file is the one that is sent.
  await submitButton(page).click();
  const stored = await page.evaluate(async () => {
    const id = new URL(location.href).searchParams.get("item");
    return (await (await fetch(`/api/v1/assignments/${id}/submission`)).json()) as { attempts: { content: { files: string[] } }[] };
  });
  expect(stored.attempts[0]!.content.files).toHaveLength(1);
});

test("student_sees_an_error_for_a_type_outside_accept", async ({ page }) => {
  await openReport(page);
  const sent = watchUploads(page);
  await picker(page).setInputFiles([doc("run.exe"), doc("fine.pdf")]);
  await expect(page.getByRole("alert").filter({ hasText: "run.exe is not a type this assignment takes. Accepted: .pdf, .docx, .png." })).toBeVisible();
  // The message comes before any upload; the file that fits is still taken.
  await expect(filesList(page)).toContainText("fine.pdf");
  await expect(filesList(page)).not.toContainText("run.exe");
  expect(sent.map((s) => s.url.searchParams.get("name"))).toEqual(["fine.pdf"]);
  // The case of an extension does not matter.
  await picker(page).setInputFiles([doc("SCAN.PNG")]);
  await expect(filesList(page)).toContainText("SCAN.PNG");
});

test("student_cannot_add_more_than_max_files", async ({ page }) => {
  await openReport(page);
  const sent = watchUploads(page);
  await picker(page).setInputFiles([doc("1.pdf"), doc("2.pdf"), doc("3.pdf")]);
  await expect(filesList(page).getByRole("listitem")).toHaveCount(3);
  await expect.poll(() => sent.length).toBe(3);
  await expect(page.getByText("Uploading…")).toHaveCount(0);
  await picker(page).setInputFiles([doc("4.pdf")]);
  await expect(page.getByRole("alert").filter({ hasText: "You can hand in at most 3 files" })).toBeVisible();
  expect(sent).toHaveLength(3);
  await expect(filesList(page).getByRole("listitem")).toHaveCount(3);
  // Two files with one name are two files.
  await page.getByRole("button", { name: "Remove 3.pdf" }).click();
  await picker(page).setInputFiles([doc("1.pdf")]);
  await expect(filesList(page).getByRole("listitem")).toHaveCount(3);
});

test("submit_is_disabled_with_no_files", async ({ page }) => {
  await openReport(page);
  await expect(submitButton(page)).toBeDisabled();
  await expect(page.getByText("Add at least one file to submit.")).toBeVisible();
  await picker(page).setInputFiles([doc("a.pdf")]);
  await expect(submitButton(page)).toBeEnabled();
  await page.getByRole("button", { name: "Remove a.pdf" }).click();
  await expect(submitButton(page)).toBeDisabled();
});

test("draft_autosave_sends_files_and_omits_an_empty_comment", async ({ page }) => {
  await openReport(page);
  const saved = watchBodies(page, "PUT", /\/submission\/draft$/);
  await picker(page).setInputFiles([doc("a.pdf")]);
  await expect.poll(() => saved.length).toBeGreaterThan(0);
  const id = await page.evaluate(async () => {
    const item = new URL(location.href).searchParams.get("item");
    return ((await (await fetch(`/api/v1/assignments/${item}/submission`)).json()) as { draft: { content: { files: string[] } } }).draft.content.files[0]!;
  });
  expect(saved.at(-1)).toEqual({ content: { files: [id] } });
  // A comment is sent once there is one.
  await page.getByRole("group", { name: "Comment, optional" }).getByRole("textbox").first().click();
  await page.keyboard.type("Sorry it is late");
  await expect.poll(() => JSON.stringify(saved.at(-1))).toContain("Sorry it is late");
  const last = saved.at(-1)!.content as { files: string[]; comment: { v: number } };
  expect(last.files).toEqual([id]);
  expect(last.comment.v).toBe(1);
});

test("a_swept_upload_shows_the_reupload_message_and_the_form_stays_usable", async ({ page }) => {
  await openReport(page);
  // The server sweeps an upload nobody named for 24 hours and then refuses a draft that names it (once here).
  await page.evaluate(() => {
    const real = window.fetch.bind(window);
    let refused = false;
    window.fetch = async (input, init) => {
      if (!refused && init?.method === "PUT" && /\/submission\/draft$/.test(String(input))) {
        refused = true;
        const problem = { title: "Unprocessable", status: 422, errors: [{ field: "files[0]", message: "the file cannot be used here" }] };
        return new Response(JSON.stringify(problem), { status: 422, headers: { "Content-Type": "application/problem+json" } });
      }
      return real(input, init);
    };
  });
  await picker(page).setInputFiles([doc("old.pdf")]);
  await expect(page.getByRole("alert").filter({ hasText: "Upload it again." })).toBeVisible();
  // The file that cannot be used is dropped from the list, and the form takes another one.
  await expect(page.getByText("old.pdf")).toHaveCount(0);
  await picker(page).setInputFiles([doc("new.pdf")]);
  await expect(filesList(page)).toContainText("new.pdf");
  await expect(submitButton(page)).toBeEnabled();
});

test("submitted_attempts_list_their_files_with_open_buttons", async ({ page }) => {
  await openReport(page);
  await picker(page).setInputFiles([doc("report.pdf"), doc("notes.docx")]);
  await expect(filesList(page).getByRole("listitem")).toHaveCount(2);
  await submitButton(page).click();
  const attempt = page.getByRole("region", { name: "Attempt 1" });
  await expect(attempt).toContainText("report.pdf");
  await expect(attempt).toContainText("notes.docx");
  await page.evaluate(() => {
    const open = window.open.bind(window);
    const calls: unknown[][] = [];
    Object.assign(window, { __opened: calls });
    window.open = (...args: Parameters<typeof window.open>) => (calls.push(args), open(...args));
  });
  await Promise.all([page.waitForEvent("popup"), attempt.getByRole("button", { name: "Open report.pdf" }).click()]);
  const [url] = (await page.evaluate(() => (window as unknown as { __opened: string[][] }).__opened))[0]!;
  expect(url).toMatch(/^\/api\/v1\/files\/[0-9a-f-]+$/);
  const [download] = await Promise.all([page.waitForEvent("download"), attempt.getByRole("button", { name: "Open notes.docx" }).click()]);
  expect(download.suggestedFilename()).toBe("notes.docx");
  // A new attempt starts with no files: the old ones do not carry over.
  await page.getByRole("button", { name: "Edit and resubmit" }).click();
  await expect(filesList(page).getByRole("listitem")).toHaveCount(0);
  await expect(submitButton(page)).toBeDisabled();
});

test("teacher_sets_max_files_and_accept_in_settings", async ({ page }) => {
  const { course, item } = await openReport(page, "teacher");
  await page.goto(`/course/item/edit?id=${course}&item=${item}`);
  const patches = watchBodies(page, "PATCH", /\/items\/[^/]+$/);
  const max = page.getByLabel("Most files per attempt");
  const accept = page.getByLabel("Accepted file types");
  await expect(max).toHaveValue("3");
  await expect(accept).toHaveValue(".pdf, .docx, .png");
  // The rules of the server are checked while typing, and nothing is sent until they hold.
  await max.fill("21");
  await expect(page.getByText("Enter a whole number from 1 to 20.")).toBeVisible();
  await accept.fill("pdf, .exe!, .PDF");
  await expect(page.getByText(".exe! is not a file type")).toBeVisible();
  await expect(page.getByRole("button", { name: "Save settings" })).toBeDisabled();
  await max.fill("2");
  // A missing dot is added and capitals are lowered; a type listed twice is refused.
  await accept.fill("PDF, docx, .pdf");
  await expect(page.getByText(".pdf is listed twice.")).toBeVisible();
  await accept.fill("PDF, docx");
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Saved" })).toBeVisible();
  const body = patches.at(-1)!;
  expect(body.config).toEqual({ max_files: 2, accept: [".pdf", ".docx"] });
  const stored = await page.evaluate(async (id) => (await (await fetch(`/api/v1/items/${id}`)).json()) as { assignment: { config: unknown } }, item);
  expect(stored.assignment.config).toEqual({ max_files: 2, accept: [".pdf", ".docx"] });
  // No restriction is an empty list left out of the config.
  await accept.fill("");
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect.poll(() => patches.length).toBe(2);
  expect(patches.at(-1)!.config).toEqual({ max_files: 2 });
});

// ---- grade mode: the teacher looks at the files a student handed in (spec 9.3) ----
// The mock holds three submissions of the seeded file assignment: Daniel's attempt 1 (report.pdf, figure.png,
// results.txt and a comment), Daniel's attempt 2 (appendix.docx, slides.pptx, diagram.svg) and Sofia's long-log.txt (1.5 MiB).

// Whether a PDF is shown in an <iframe> (see PDF_IFRAME_PREVIEW in FilePreview.tsx) or as the Open card; the specs follow it.
const PDF_IFRAME = /PDF_IFRAME_PREVIEW = true/.test(readFileSync(new URL("../src/components/grading/FilePreview.tsx", import.meta.url), "utf8"));
const MIB = 1_048_576;

/** Signs in as the teacher and opens the grading workspace on `student`'s submission of the file assignment. */
async function gradeFiles(page: Page, student: "Daniel" | "Sofia", init?: string) {
  if (init) await page.addInitScript(init);
  await signInAs(page, "teacher");
  const ids = await report(page);
  const id = await page.evaluate(async (args) => {
    const rows = (await (await fetch(`/api/v1/assignments/${args.item}/submissions?limit=100`)).json()) as { items: { name: string; student_id: string }[] };
    return rows.items.find((r) => r.name === args.student)!.student_id;
  }, { item: ids.item, student });
  await page.goto(gradeHref(ids.course, ids.item, id));
  const region = page.getByRole("region", { name: "Submission" });
  await expect(region.getByRole("list", { name: "Files" })).toBeVisible();
  return region;
}

const fileButton = (region: ReturnType<Page["getByRole"]>, name: string) => region.getByRole("list", { name: "Files" }).getByRole("button", { name });
const preview = (region: ReturnType<Page["getByRole"]>, name: string) => region.getByRole("region", { name: `Preview of ${name}` });

/** Daniel's older attempt, which holds the PDF, the image and the text. */
async function firstAttempt(page: Page) {
  const region = await gradeFiles(page, "Daniel");
  await region.getByRole("tab", { name: "Attempt 1" }).click();
  await expect(fileButton(region, "report.pdf")).toBeVisible();
  return region;
}

test("teacher_previews_a_pdf_per_the_step0_decision", async ({ page }) => {
  const region = await firstAttempt(page);
  const shown = preview(region, "report.pdf");
  if (PDF_IFRAME) {
    await expect(shown.locator("iframe")).toHaveAttribute("src", /^\/api\/v1\/files\/[0-9a-f-]+$/);
    await expect(shown.locator("iframe")).toHaveAttribute("title", "report.pdf");
    await expect(shown.getByRole("button", { name: "Open in new tab" })).toHaveCount(0);
  } else {
    // The sandbox policy the server sends keeps Chrome from drawing a PDF in a frame: the card opens it in a tab.
    await expect(shown.locator("iframe")).toHaveCount(0);
    await expect(shown.getByRole("group", { name: "File" })).toContainText("report.pdf");
    const [popup] = await Promise.all([page.waitForEvent("popup"), shown.getByRole("button", { name: "Open in new tab" }).click()]);
    await popup.close();
  }
});

test("teacher_previews_an_image", async ({ page }) => {
  const region = await firstAttempt(page);
  await fileButton(region, "figure.png").click();
  const img = preview(region, "figure.png").getByRole("img", { name: "figure.png" });
  await expect(img).toBeVisible();
  await expect.poll(() => img.evaluate((el) => (el as HTMLImageElement).naturalWidth)).toBeGreaterThan(0);
  await expect(img).toHaveAttribute("src", /^\/api\/v1\/files\/[0-9a-f-]+$/);
});

test("teacher_previews_text_up_to_1_mib_and_sees_a_truncation_note_beyond", async ({ page }) => {
  const small = await firstAttempt(page);
  await fileButton(small, "results.txt").click();
  await expect(preview(small, "results.txt").locator("pre")).toContainText("trial 1: 9.81");
  await expect(preview(small, "results.txt").getByText("Showing only the first 1 MiB")).toHaveCount(0);
  // Another student's 1.5 MiB log: the first MiB is shown and the note says the rest is not.
  const big = await gradeFiles(page, "Sofia");
  const pre = preview(big, "long-log.txt").locator("pre");
  await expect(pre).toContainText("line 1");
  await expect(preview(big, "long-log.txt").getByText("Showing only the first 1 MiB")).toBeVisible();
  expect(await pre.evaluate((el) => el.textContent!.length)).toBe(MIB);
});

test("teacher_sees_an_open_card_for_a_pptx", async ({ page }) => {
  const region = await gradeFiles(page, "Daniel");
  await fileButton(region, "slides.pptx").click();
  const shown = preview(region, "slides.pptx");
  await expect(shown.getByRole("group", { name: "File" })).toContainText("slides.pptx");
  await expect(shown.locator("iframe, img, pre")).toHaveCount(0);
  const [download] = await Promise.all([page.waitForEvent("download"), shown.getByRole("button", { name: "Open", exact: true }).click()]);
  expect(download.suggestedFilename()).toBe("slides.pptx");
});

// Tauri's window, enough for the grading page: the API commands go to the mock, and `open_file` is recorded.
const desktopOn = (userAgent: string) => `
  Object.defineProperty(navigator, "userAgent", { get: () => ${JSON.stringify(userAgent)} });
  window.__calls = [];
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 1,
    unregisterCallback() {},
    convertFileSrc: (id) => "/api/v1/files/" + id,
    async invoke(cmd, args) {
      window.__calls.push({ cmd, args });
      if (cmd === "api") {
        const res = await fetch("/api/v1" + args.path, { method: args.method, headers: args.body ? { "content-type": "application/json" } : undefined, body: args.body ? JSON.stringify(args.body) : undefined });
        const text = res.status === 204 ? "" : await res.text();
        let body = text ? JSON.parse(text) : null;
        if (args.path === "/auth/login" && res.ok && body && body.user) body = body.user;
        return { status: res.status, body, from_cache: false, cached_at: null, queued: false };
      }
      if (cmd === "sync_state") return { online: true, pending: 0, failed: 0, paused: false, last_ok: Date.now(), sent: 0, queued: [] };
      if (cmd === "outbox_list") return [];
      if (cmd === "open_file") return "opened";
      return null;
    },
  };
`;
const LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko)";

test("desktop_linux_shows_an_open_card_for_a_pdf", async ({ page }) => {
  const region = await gradeFiles(page, "Daniel", desktopOn(LINUX));
  await region.getByRole("tab", { name: "Attempt 1" }).click();
  const shown = preview(region, "report.pdf");
  // WebKitGTK cannot draw a PDF; the card hands it to the system's program.
  await expect(shown.getByRole("group", { name: "File" })).toContainText("report.pdf");
  await expect(shown.locator("iframe")).toHaveCount(0);
  await shown.getByRole("button", { name: "Open", exact: true }).click();
  const calls = await page.evaluate(() => (window as unknown as { __calls: { cmd: string }[] }).__calls);
  expect(calls.filter((c) => c.cmd === "open_file")).toHaveLength(1);
});

test("desktop_on_another_system_previews_a_pdf_when_the_iframe_is_enabled", async ({ page }) => {
  test.skip(!PDF_IFRAME, "PDF_IFRAME_PREVIEW is off: every PDF is an Open card");
  const region = await gradeFiles(page, "Daniel", desktopOn(MAC));
  await region.getByRole("tab", { name: "Attempt 1" }).click();
  await expect(preview(region, "report.pdf").locator("iframe")).toHaveAttribute("src", /\/api\/v1\/files\//);
});

test("switching_attempt_tabs_switches_the_file_list", async ({ page }) => {
  const region = await gradeFiles(page, "Daniel");
  // The newest attempt is shown first.
  const list = region.getByRole("list", { name: "Files" });
  await expect(list).toContainText("appendix.docx");
  await expect(list).toContainText("slides.pptx");
  await expect(list).not.toContainText("report.pdf");
  await region.getByRole("tab", { name: "Attempt 1" }).click();
  await expect(list).toContainText("report.pdf");
  await expect(list).toContainText("results.txt");
  await expect(list).not.toContainText("slides.pptx");
  await region.getByRole("tab", { name: "Attempt 2" }).click();
  await expect(list).toContainText("slides.pptx");
  await expect(list).not.toContainText("results.txt");
});

test("comment_is_shown_under_the_files", async ({ page }) => {
  const region = await firstAttempt(page);
  const comment = region.getByText("The figures are in the PNG");
  await expect(comment).toBeVisible();
  const list = (await region.getByRole("list", { name: "Files" }).boundingBox())!;
  const note = (await comment.boundingBox())!;
  expect(note.y).toBeGreaterThan(list.y + list.height - 1);
  // The newer attempt has no comment.
  await region.getByRole("tab", { name: "Attempt 2" }).click();
  await expect(comment).toHaveCount(0);
});

test("svg_is_never_previewed_inline", async ({ page }) => {
  const region = await gradeFiles(page, "Daniel");
  await fileButton(region, "diagram.svg").click();
  const shown = preview(region, "diagram.svg");
  await expect(shown.getByRole("group", { name: "File" })).toContainText("diagram.svg");
  await expect(shown.locator("img, iframe, object, embed, pre")).toHaveCount(0);
  await expect(shown.getByRole("button", { name: "Open", exact: true })).toBeVisible();
});
