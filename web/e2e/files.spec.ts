import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD } from "./routes";

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
