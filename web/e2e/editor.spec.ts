import { expect, test, type Page } from "@playwright/test";
import { accounts, courseRoutes, PASSWORD } from "./routes";
import { blockMenu, menuOffers } from "../src/components/doc/model";

async function openLectureEditor(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.teacher);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  const edit = (await courseRoutes(page, "teacher")).find((r) => r.startsWith("/course/item/edit"));
  expect(edit, "an item to edit").toBeTruthy();
  await page.goto(edit!);
  const last = page.locator("[data-editable]").last();
  await last.click();
  await page.keyboard.press("Control+End");
  await page.keyboard.press("Enter");
}

/** The student's draft answer of "Merge sort implementation": a paragraph and a code block, which is last. */
async function openAnswer(page: Page) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts.student);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  const href = await page.evaluate(async () => {
    const courses = await (await fetch("/api/v1/me/courses?limit=100")).json();
    for (const c of courses.items) {
      const tree = await (await fetch(`/api/v1/courses/${c.id}/content`)).json();
      const items = tree.modules.flatMap((m: { items: { id: string; title: string }[]; submodules: { items: { id: string; title: string }[] }[] }) => [...m.items, ...m.submodules.flatMap((s) => s.items)]);
      const hit = items.find((i: { title: string }) => i.title === "Merge sort implementation");
      if (hit) return `/course/item?id=${c.id}&item=${hit.id}`;
    }
    return null;
  });
  expect(href, "the assignment with a draft").toBeTruthy();
  await page.goto(href!);
  await expect(page.locator("[data-editable]").first()).toBeVisible();
}

/** A click on the empty page under the last block of the answer. */
async function clickBelowLastBlock(page: Page) {
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  const box = (await page.getByRole("group", { name: "Your answer" }).boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height - 16);
}

test("a click below the last block goes on writing there, even after a code block", async ({ page }) => {
  await openAnswer(page);
  const blocks = page.locator("[data-block-id]");
  const before = await blocks.count();
  await clickBelowLastBlock(page);
  await page.keyboard.type("and then more");
  await expect(blocks).toHaveCount(before + 1);
  await expect(page.locator("[data-editable]").last()).toHaveText("and then more");
});

test("a formula is drawn by KaTeX, and Enter leaves the formula field for the text after it", async ({ page }) => {
  await openAnswer(page);
  await clickBelowLastBlock(page);
  await page.keyboard.type("/formula");
  await page.keyboard.press("Enter");
  await page.keyboard.type("\\int_{1}^{2} x \\, dx");
  await expect(page.locator(".math .katex-html").last()).toContainText("∫");
  await page.keyboard.press("Enter");
  await page.keyboard.type("after");
  await expect(page.locator("[data-editable]").filter({ hasText: "after" })).toHaveCount(1);
});

test("the block menu opened with the + of the gutter is filtered by what is typed", async ({ page }) => {
  await openAnswer(page);
  const first = page.locator("[data-editable]").first();
  await first.hover();
  await page.getByRole("button", { name: "Add a block" }).first().click({ force: true });
  await expect(page.getByRole("listbox")).toBeVisible();
  await page.keyboard.type("numbered");
  await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(1);
  await page.keyboard.press("Enter");
  await expect(page.locator("[data-block-id]").getByText("1.", { exact: true })).toBeVisible();
});

test("typing / opens the block menu at once", async ({ page }) => {
  await openLectureEditor(page);
  await page.keyboard.type("/");
  await expect(page.getByRole("listbox")).toBeVisible();
  await page.keyboard.type(" ");
  await expect(page.getByRole("listbox")).toHaveCount(0);
});

test("a list marker typed as a shortcut is not kept as text", async ({ page }) => {
  await openLectureEditor(page);
  await page.keyboard.type("- a point");
  const item = page.locator("[data-editable]").last();
  await expect(item).toHaveText("a point");
  await expect(page.locator("[data-block-id]").last().getByText("•")).toBeVisible();
});

// ---- images and files (ADR-0069, spec 6 and 9.2) ----

const PNG_B64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const PDF_B64 = Buffer.from("%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n").toString("base64");
type Pick = { name: string; type: string; b64: string };
const PNG_FILE: Pick = { name: "diagram.png", type: "image/png", b64: PNG_B64 };
const PDF_FILE: Pick = { name: "lab.pdf", type: "application/pdf", b64: PDF_B64 };

/** A paste of `files` (and `text`) into the focused element, as the browser would deliver it. */
async function pasteInto(page: Page, files: Pick[], text?: string) {
  await page.evaluate(
    ([list, plain]) => {
      const dt = new DataTransfer();
      for (const f of list as Pick[]) dt.items.add(new File([Uint8Array.from(atob(f.b64), (c) => c.charCodeAt(0))], f.name, { type: f.type }));
      if (plain) dt.setData("text/plain", plain as string);
      document.activeElement!.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
    },
    [files, text ?? ""] as const,
  );
}

/** A drop of `files` onto the last block of the editor. */
async function dropOnLastBlock(page: Page, files: Pick[]) {
  await page.evaluate((list) => {
    const dt = new DataTransfer();
    for (const f of list) dt.items.add(new File([Uint8Array.from(atob(f.b64), (c) => c.charCodeAt(0))], f.name, { type: f.type }));
    const blocks = document.querySelectorAll("[data-block-id]");
    blocks[blocks.length - 1]!.dispatchEvent(new DragEvent("drop", { dataTransfer: dt, bubbles: true, cancelable: true }));
  }, files);
}

/** The documents the page has sent for the item so far (PATCH /items/:id), as they arrive. */
type SavedBlock = { type: string; props?: Record<string, unknown> };
function watchSaves(page: Page): { blocks: SavedBlock[] }[] {
  const docs: { blocks: SavedBlock[] }[] = [];
  page.on("request", (r) => {
    if (r.method() === "PATCH" && /\/api\/v1\/items\//.test(r.url())) {
      const body = r.postDataJSON() as { body?: { blocks: SavedBlock[] } };
      if (body.body) docs.push(body.body);
    }
  });
  return docs;
}

test("slash_menu_offers_image_and_file_only_with_an_upload_target", async ({ page }) => {
  const names = (canUpload: boolean, features: string[]) => blockMenu().flatMap((g) => g.items).filter((i) => menuOffers(i, features, canUpload)).map((i) => i.name);
  expect(names(false, ["image", "file"])).not.toContain("Image");
  expect(names(false, ["image", "file"])).not.toContain("File");
  expect(names(true, ["image"])).toContain("Image");
  expect(names(true, ["image"])).not.toContain("File");
  // The question and option profiles take pictures but no files.
  expect(names(true, ["text_styles", "code", "math", "image"])).toEqual(expect.arrayContaining(["Image"]));
  expect(names(true, ["text_styles", "code", "math", "image"])).not.toContain("File");
  await openLectureEditor(page);
  await page.keyboard.type("/");
  const menu = page.getByRole("listbox");
  await expect(menu.getByRole("option", { name: /^Image/ })).toBeVisible();
  await expect(menu.getByRole("option", { name: /^File/ })).toBeVisible();
});

test("pasting_an_image_uploads_and_inserts_an_image_block", async ({ page }) => {
  await openLectureEditor(page);
  const saves = watchSaves(page);
  await pasteInto(page, [PNG_FILE]);
  const img = page.locator("[data-block-id] img");
  await expect(img).toBeVisible();
  await expect(img).toHaveAttribute("src", /\/api\/v1\/files\/[0-9a-f-]+$/);
  await expect.poll(() => img.evaluate((e: HTMLImageElement) => e.naturalWidth)).toBeGreaterThan(0);
  const id = (await img.getAttribute("src"))!.split("/").pop()!;
  // The saved document names the stored file in `props.file_id` and nothing else (no alt or caption was typed).
  await expect.poll(() => saves.flatMap((d) => d.blocks).find((b) => b.type === "image")?.props).toEqual({ file_id: id });
});

test("dropping_a_pdf_inserts_a_file_card", async ({ page }) => {
  await openLectureEditor(page);
  const saves = watchSaves(page);
  await dropOnLastBlock(page, [PDF_FILE]);
  const card = page.getByRole("group", { name: "File", exact: true });
  await expect(card).toBeVisible();
  await expect(card).toContainText("lab.pdf");
  await expect(card.getByRole("button", { name: "Open" })).toBeVisible();
  await expect.poll(() => saves.flatMap((d) => d.blocks).find((b) => b.type === "file")?.props).toEqual({ file_id: expect.stringMatching(/^[0-9a-f-]+$/) });
});

test("a_placeholder_shows_while_uploading_and_is_replaced", async ({ page }) => {
  await openLectureEditor(page);
  await page.evaluate(() => window.__vumloFiles.setUploadDelay(1500));
  await pasteInto(page, [PNG_FILE]);
  const busy = page.locator('[aria-busy="true"]');
  await expect(busy).toBeVisible();
  await expect(busy).toContainText("diagram.png");
  await expect(page.locator("[data-block-id] img")).toBeVisible();
  await expect(busy).toHaveCount(0);
});

test("a_failed_upload_removes_the_block_and_shows_a_toast", async ({ page }) => {
  await openLectureEditor(page);
  await page.evaluate(() => window.__vumloFiles.setMaxSize(10));
  await pasteInto(page, [PNG_FILE]);
  await expect(page.getByRole("status").filter({ hasText: "This file is too large." })).toBeVisible();
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  await expect(page.locator("[data-block-id] img")).toHaveCount(0);
});

test("dropping_a_file_inserts_it_after_the_block_under_the_drop", async ({ page }) => {
  await openLectureEditor(page);
  await page.evaluate((f) => {
    const dt = new DataTransfer();
    dt.items.add(new File([Uint8Array.from(atob(f.b64), (c) => c.charCodeAt(0))], f.name, { type: f.type }));
    document.querySelector("[data-block-id]")!.dispatchEvent(new DragEvent("drop", { dataTransfer: dt, bubbles: true, cancelable: true }));
  }, PDF_FILE);
  await expect(page.getByRole("group", { name: "File", exact: true })).toBeVisible();
  const kinds = await page.locator("[data-block-id]").evaluateAll((els) => els.map((e) => (e.querySelector("[aria-label='File']") ? "file" : "other")));
  expect(kinds.indexOf("file")).toBe(1);
});

test("several_files_upload_in_order", async ({ page }) => {
  await openLectureEditor(page);
  await pasteInto(page, [PDF_FILE, PNG_FILE]);
  await expect(page.locator("[data-block-id] img")).toBeVisible();
  const kinds = await page.locator("[data-block-id]").evaluateAll((els) => els.map((e) => (e.querySelector("img") ? "image" : e.querySelector("[aria-label='File']") ? "file" : "text")));
  expect(kinds.filter((k) => k !== "text")).toEqual(["file", "image"]);
});

/** "Image" from the slash menu, then the file the person picks in the chooser the menu opens. */
async function chooseImage(page: Page, file: { name: string; mimeType: string; buffer: Buffer }) {
  await page.keyboard.type("/image");
  await expect(page.getByRole("option")).toHaveCount(1);
  // The listener must be on before the key: Playwright turns the interception of the dialog on when it is added.
  const opened = page.waitForEvent("filechooser");
  await page.evaluate(() => undefined);
  await page.keyboard.press("Enter");
  await (await opened).setFiles(file);
}

/** The published lecture of the mock course that has an image and a file card (src/mocks/seed.ts). */
async function openSeededLecture(page: Page, who: "student" | "teacher") {
  await page.goto("/login");
  await page.getByLabel("Email").fill(accounts[who]);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"));
  const href = await page.evaluate(async () => {
    const courses = await (await fetch("/api/v1/me/courses?limit=100")).json();
    for (const c of courses.items) {
      const tree = await (await fetch(`/api/v1/courses/${c.id}/content`)).json();
      const items = tree.modules.flatMap((m: { items: { id: string; title: string }[]; submodules: { items: { id: string; title: string }[] }[] }) => [...m.items, ...m.submodules.flatMap((s) => s.items)]);
      const hit = items.find((i: { title: string }) => i.title === "Big-O in practice");
      if (hit) return `/course/item?id=${c.id}&item=${hit.id}`;
    }
    return null;
  });
  expect(href, "the lecture with an image and a file").toBeTruthy();
  await page.goto(href!);
}

test("renderer_shows_an_image_and_a_file_card_that_opens", async ({ page }) => {
  await openSeededLecture(page, "student");
  const img = page.getByRole("img", { name: "Merge sort splits the list in half" });
  await expect(img).toBeVisible();
  await expect.poll(() => img.evaluate((e: HTMLImageElement) => e.naturalWidth)).toBeGreaterThan(0);
  await expect(page.getByText("Picture: how the list is split")).toBeVisible();
  const card = page.getByRole("group", { name: "File", exact: true });
  await expect(card).toContainText("lab-guide.pdf");
  await page.evaluate(() => {
    const open = window.open.bind(window);
    Object.assign(window, { __opened: [] as unknown[][] });
    window.open = (...args: Parameters<typeof window.open>) => ((window as unknown as { __opened: unknown[][] }).__opened.push(args), open(...args));
  });
  await Promise.all([page.waitForEvent("popup"), card.getByRole("button", { name: "Open" }).click()]);
  const [opened] = await page.evaluate(() => (window as unknown as { __opened: string[][] }).__opened);
  expect(opened![0]).toMatch(/^\/api\/v1\/files\//);
});

test("an_image_that_cannot_be_loaded_shows_a_placeholder", async ({ page }) => {
  await openLectureEditor(page);
  // Not a picture: the image block keeps the file, the page shows a neutral box instead of a broken image.
  await chooseImage(page, { name: "notes.pdf", mimeType: "application/pdf", buffer: Buffer.from("%PDF-1.4") });
  await expect(page.getByText("This picture cannot be shown.")).toBeVisible();
});

test("student_editor_has_image_and_file_but_question_editor_has_no_file", async ({ page }) => {
  await openAnswer(page);
  await page.locator("[data-editable]").last().click();
  await page.keyboard.press("Control+End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("/");
  const menu = page.getByRole("listbox");
  await expect(menu.getByRole("option", { name: /^Image/ })).toBeVisible();
  await expect(menu.getByRole("option", { name: /^File/ })).toBeVisible();
  await page.keyboard.press("Escape");
  const profiles = await page.evaluate(async () => (await (await fetch("/api/v1/editor/profiles")).json()) as { profiles: { name: string; features: string[] }[] });
  const has = (name: string, f: string) => profiles.profiles.find((p) => p.name === name)!.features.includes(f);
  expect([has("student", "image"), has("student", "file")]).toEqual([true, true]);
  expect([has("question", "image"), has("question", "file")]).toEqual([true, false]);
  expect([has("option", "image"), has("option", "file")]).toEqual([true, false]);
  // A student's own upload goes to the assignment.
  await pasteInto(page, [PDF_FILE]);
  await expect(page.getByRole("group", { name: "File", exact: true })).toContainText("lab.pdf");
});

test("an_image_block_naming_a_pdf_shows_the_server_message_on_that_block", async ({ page }) => {
  await openLectureEditor(page);
  await chooseImage(page, { name: "notes.pdf", mimeType: "application/pdf", buffer: Buffer.from("%PDF-1.4") });
  const block = page.locator("[data-block-id]").filter({ has: page.locator("[data-kind-block=image]") });
  await expect(block.getByRole("alert")).toHaveText("an image block needs a PNG, JPEG, GIF or WebP file", { timeout: 15_000 });
});

test("the_saved_document_never_contains_a_placeholder", async ({ page }) => {
  await openLectureEditor(page);
  const saves = watchSaves(page);
  await page.evaluate(() => window.__vumloFiles.setUploadDelay(3000));
  await pasteInto(page, [PNG_FILE]);
  await expect(page.locator('[aria-busy="true"]')).toBeVisible();
  await page.locator("[data-editable]").first().click();
  await page.keyboard.type(" typed while uploading");
  await expect.poll(() => saves.length, { timeout: 10_000 }).toBeGreaterThan(0);
  await expect(page.locator('[aria-busy="true"]')).toBeVisible();
  const known = new Set(["paragraph", "heading", "quote", "bulleted_list_item", "numbered_list_item", "check_list_item", "divider", "code", "math", "mermaid", "video", "table", "image", "file"]);
  expect(saves.flatMap((d) => d.blocks).filter((b) => !known.has(b.type))).toEqual([]);
  expect(saves.every((d) => d.blocks.every((b) => b.type !== "image"))).toBe(true);
  await expect(page.locator("[data-block-id] img")).toBeVisible();
});

test("a_swept_upload_shows_the_reupload_message_on_the_block", async ({ page }) => {
  await openLectureEditor(page);
  await pasteInto(page, [PNG_FILE]);
  const img = page.locator("[data-block-id] img");
  await expect(img).toBeVisible();
  const id = (await img.getAttribute("src"))!.split("/").pop()!;
  await page.evaluate((f) => window.__vumloFiles.sweep(f), id);
  await page.locator("[data-editable]").first().click();
  await page.keyboard.type(" and more");
  const block = page.locator("[data-block-id]").filter({ has: page.locator("[data-kind-block=image]") });
  await expect(block.getByRole("alert")).toHaveText("The uploaded file is no longer stored. Upload it again.", { timeout: 15_000 });
  // The editor stays usable.
  await page.locator("[data-editable]").first().click();
  await page.keyboard.type(" still typing");
  await expect(page.locator("[data-editable]").first()).toContainText("still typing");
});

test("pasting_plain_text_still_works", async ({ page }) => {
  await openLectureEditor(page);
  await pasteInto(page, [], "just words");
  await expect(page.locator("[data-editable]").last()).toHaveText("just words");
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
});

test("replacing_the_file_of_an_image_block_swaps_the_picture", async ({ page }) => {
  await openLectureEditor(page);
  await pasteInto(page, [PNG_FILE]);
  const img = page.locator("[data-block-id] img");
  await expect(img).toBeVisible();
  const before = await img.getAttribute("src");
  const opened = page.waitForEvent("filechooser");
  await page.evaluate(() => undefined);
  await page.locator("[data-kind-block=image]").getByRole("button", { name: "Replace" }).click();
  await (await opened).setFiles({ name: "other.png", mimeType: "image/png", buffer: Buffer.from(PNG_B64, "base64") });
  await expect(img).not.toHaveAttribute("src", before!);
  await expect(page.locator("[data-block-id] img")).toHaveCount(1);
});
