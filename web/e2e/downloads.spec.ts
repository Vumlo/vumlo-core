import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD } from "./routes";

// The links to the institution's desktop app (ADR-0064): the notice on the home page, the download page and the
// admin's form. The mock institution has a Windows, an Apple-silicon and a .deb link.
const WINDOWS = "https://downloads.northfield.edu/northfield-setup.exe";
const MAC = "https://downloads.northfield.edu/northfield-arm64.dmg";
const DEB = "https://downloads.northfield.edu/northfield.deb";

const agents = {
  windows: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
  mac: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
  linux: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
  phone: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
};

async function signIn(page: Page, email: string) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
}

const island = (page: Page) => page.getByRole("region", { name: "Get the Northfield app" });

test.describe("on Windows", () => {
  test.use({ userAgent: agents.windows });

  test("the home page offers the installer for this system, and it can be hidden for good", async ({ page }) => {
    await signIn(page, accounts.student);
    await expect(island(page)).toBeVisible();
    await expect(island(page).getByRole("link", { name: "Download for Windows" })).toHaveAttribute("href", WINDOWS);
    await island(page).getByRole("link", { name: "All downloads" }).click();
    await expect(page).toHaveURL(/\/download$/);
  });

  test("a teacher sees it too, and hiding it lasts", async ({ page }) => {
    await signIn(page, accounts.teacher);
    await expect(island(page)).toBeVisible();
    await island(page).getByRole("button", { name: "Hide this notice" }).click();
    await expect(island(page)).toHaveCount(0);
    await page.reload();
    await signIn(page, accounts.teacher);
    await expect(page.getByRole("heading", { name: /Good (morning|afternoon|evening)/ })).toBeVisible();
    await expect(island(page)).toHaveCount(0);
  });

  test("the download page needs no sign-in and puts this computer's system first", async ({ page }) => {
    await page.goto("/download");
    await expect(page.getByRole("heading", { name: "Get the Northfield app" })).toBeVisible();
    const headings = await page.getByRole("heading", { level: 2 }).allTextContents();
    expect(headings).toEqual(["Windows", "macOS", "Linux"]);
    await expect(page.getByText("For this computer")).toHaveCount(1);
    await expect(page.getByRole("link", { name: "Windows installer (.exe)" })).toHaveAttribute("href", WINDOWS);
    await expect(page.getByRole("link", { name: "macOS, Apple silicon (.dmg)" })).toHaveAttribute("href", MAC);
    await expect(page.getByRole("link", { name: "Linux, Ubuntu and Debian (.deb)" })).toHaveAttribute("href", DEB);
    await expect(page.getByRole("link", { name: "macOS, Intel (.dmg)" })).toHaveCount(0);
    await expect(page.getByRole("link", { name: "Linux, AppImage" })).toHaveCount(0);
  });

  test("the account menu has the page", async ({ page }) => {
    await signIn(page, accounts.student);
    await page.getByRole("button", { name: /^Account menu/ }).click();
    await page.getByRole("menuitem", { name: "Get the desktop app" }).click();
    await expect(page).toHaveURL(/\/download$/);
  });
});

test.describe("on a Mac", () => {
  test.use({ userAgent: agents.mac });
  test("the notice goes straight to the installer when there is one for the system", async ({ page }) => {
    await signIn(page, accounts.student);
    await expect(island(page).getByRole("link", { name: "Download for macOS" })).toHaveAttribute("href", MAC);
  });
  test("the download page lists macOS first", async ({ page }) => {
    await page.goto("/download");
    await expect(page.getByRole("heading", { name: "Get the Northfield app" })).toBeVisible();
    expect(await page.getByRole("heading", { level: 2 }).allTextContents()).toEqual(["macOS", "Windows", "Linux"]);
  });
});

test.describe("on Linux", () => {
  test.use({ userAgent: agents.linux });
  test("the notice offers the .deb", async ({ page }) => {
    await signIn(page, accounts.student);
    await expect(island(page).getByRole("link", { name: "Download for Linux" })).toHaveAttribute("href", DEB);
  });
});

test.describe("on a phone", () => {
  test.use({ userAgent: agents.phone });
  test("there is no notice: the app is for computers", async ({ page }) => {
    await signIn(page, accounts.student);
    await expect(page.getByRole("heading", { name: /Good (morning|afternoon|evening)/ })).toBeVisible();
    await expect(island(page)).toHaveCount(0);
  });
});

test.describe("the admin", () => {
  test.use({ userAgent: agents.windows });

  test("sets the links, is told when one is not an https address, and can switch it all off", async ({ page }) => {
    await signIn(page, accounts.admin);
    await page.getByRole("link", { name: "Institution" }).click();
    await expect(page.getByRole("heading", { name: "Desktop app" })).toBeVisible();
    await expect(page.getByLabel("Windows installer (.exe)")).toHaveValue(WINDOWS);

    await page.getByLabel("macOS, Intel (.dmg)").fill("http://downloads.northfield.edu/intel.dmg");
    await page.getByRole("button", { name: "Save links" }).click();
    await expect(page.getByRole("alert").filter({ hasText: "must be an address that starts with https://" })).toBeVisible();

    const intel = "https://downloads.northfield.edu/northfield-intel.dmg";
    await page.getByLabel("macOS, Intel (.dmg)").fill(intel);
    await page.getByRole("button", { name: "Save links" }).click();
    await expect(page.getByText("Saved. People can see the links now.")).toBeVisible();
    await page.getByRole("link", { name: "Open the download page" }).click();
    await expect(page.getByRole("link", { name: "macOS, Intel (.dmg)" })).toHaveAttribute("href", intel);

    await page.goBack();
    for (const label of ["Windows installer (.exe)", "macOS, Apple silicon (.dmg)", "macOS, Intel (.dmg)", "Linux, Ubuntu and Debian (.deb)"]) {
      await page.getByLabel(label).fill("");
    }
    await page.getByRole("button", { name: "Save links" }).click();
    await expect(page.getByText("Saved. No links: the notice and the download page are off.")).toBeVisible();
    await page.getByRole("link", { name: "Open the download page" }).click();
    await expect(page.getByRole("heading", { name: "There is no app to download yet" })).toBeVisible();
  });
});
