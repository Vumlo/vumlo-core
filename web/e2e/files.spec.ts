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
