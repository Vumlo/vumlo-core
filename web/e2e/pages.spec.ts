import { expect, test, type Page } from "@playwright/test";
import { accounts, PASSWORD } from "./routes";

async function signIn(page: Page, email: string, password: string) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
}

test.describe("accounts", () => {
  test("registering needs a confirmed university email before the first sign-in", async ({ page }) => {
    await page.goto("/register");
    await page.getByLabel("First name").fill("Nina");
    await page.getByLabel("Last name").fill("Shevchenko");
    await page.getByLabel("University email").fill("nina@gmail.com");
    await page.getByLabel("Password", { exact: true }).fill("a long unrelated phrase");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page.getByText("ending in @northfield.edu")).toBeVisible();

    await page.getByLabel("University email").fill("nina.shevchenko@northfield.edu");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page.getByRole("heading", { name: "Check your email" })).toBeVisible();

    // The account does not exist until the link is followed.
    await page.getByRole("link", { name: "Confirm the email address" }).click();
    await expect(page.getByRole("heading", { name: "Email confirmed" })).toBeVisible();
    await page.getByRole("link", { name: "Sign in" }).click();
    await page.getByLabel("Email").fill("nina.shevchenko@northfield.edu");
    await page.getByLabel("Password", { exact: true }).fill("a long unrelated phrase");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("heading", { name: /Good (morning|afternoon|evening), Nina/ })).toBeVisible();
  });

  test("an email link works once", async ({ page }) => {
    await page.goto("/verify-email?token=nothing");
    await expect(page.getByRole("heading", { name: "This link doesn't work" })).toBeVisible();
    // The reset form can only learn that its token is spent when it is submitted.
    await page.goto("/reset-password?token=nothing");
    await page.getByLabel("New password", { exact: true }).fill("a long unrelated phrase");
    await page.getByLabel("Repeat new password").fill("a long unrelated phrase");
    await page.getByRole("button", { name: "Save the password" }).click();
    await expect(page.getByRole("heading", { name: "This link doesn't work" })).toBeVisible();
  });

  test("resetting a password gives the same answer for every address, and the new password works", async ({ page }) => {
    await page.goto("/forgot-password");
    await page.getByLabel("Email").fill("nobody@northfield.edu");
    await page.getByRole("button", { name: "Send the link" }).click();
    await expect(page.getByRole("heading", { name: "Check your email" })).toBeVisible();
    await expect(page.getByText("No mail for this address.")).toBeVisible();

    await page.goto("/forgot-password");
    await page.getByLabel("Email").fill(accounts.student);
    await page.getByRole("button", { name: "Send the link" }).click();
    await expect(page.getByRole("heading", { name: "Check your email" })).toBeVisible();
    await page.getByRole("link", { name: "Choose a new password" }).click();
    await page.getByLabel("New password", { exact: true }).fill("another long unrelated phrase");
    await page.getByLabel("Repeat new password").fill("another long unrelated phrase");
    await page.getByRole("button", { name: "Save the password" }).click();
    await expect(page.getByRole("heading", { name: "Password changed" })).toBeVisible();
    await page.getByRole("link", { name: "Sign in" }).click();
    await page.getByLabel("Email").fill(accounts.student);
    await page.getByLabel("Password", { exact: true }).fill("another long unrelated phrase");
    await page.getByRole("button", { name: "Sign in", exact: true }).click();
    await expect(page.getByRole("heading", { name: /Good (morning|afternoon|evening), Maya/ })).toBeVisible();
  });

  test("signing in with Google lands on the home page", async ({ page }) => {
    await page.goto("/login");
    await page.getByLabel("Email").fill(accounts.student);
    await page.getByRole("button", { name: "Continue with Google" }).click();
    await expect(page.getByRole("heading", { name: /Good (morning|afternoon|evening), Maya/ })).toBeVisible();
  });

  test("a Google account outside the university is refused", async ({ page }) => {
    await page.goto("/login");
    await page.getByLabel("Email").fill("someone@gmail.com");
    await page.getByRole("button", { name: "Continue with Google" }).click();
    await expect(page.getByRole("heading", { name: "Google sign-in didn't work" })).toBeVisible();
  });

  test("a temporary password opens nothing but the page that replaces it", async ({ page }) => {
    await signIn(page, "omar.haddad@northfield.edu", "temporary password");
    await expect(page).toHaveURL(/\/change-password/);
    await page.goto("/courses");
    await expect(page).toHaveURL(/\/change-password/);
    // A reload forgets the temporary password that was typed, so it is asked for again.
    await page.getByLabel("Temporary password").fill("temporary password");
    await page.getByLabel("New password", { exact: true }).fill("a long unrelated phrase");
    await page.getByLabel("Repeat new password").fill("a long unrelated phrase");
    await page.getByRole("button", { name: "Save and continue" }).click();
    // It continues to the page that was asked for.
    await expect(page).toHaveURL(/\/courses$/);
    await expect(page.getByRole("heading", { name: "Courses", level: 1 })).toBeVisible();
  });
});

test.describe("missing pages", () => {
  test("an unknown address, course and item are all 'not found'", async ({ page }) => {
    await signIn(page, accounts.student, PASSWORD);
    await expect(page.getByRole("navigation", { name: "Main" }).first()).toBeVisible();
    for (const path of ["/no-such-page", "/course", "/course?id=nothing", "/course/item?id=nothing&item=nothing"]) {
      await page.goto(path);
      await expect(page.getByRole("heading", { name: "We can't find that page" })).toBeVisible();
    }
    await page.getByRole("link", { name: "Go to Home" }).click();
    await expect(page.getByRole("heading", { name: /Good (morning|afternoon|evening)/ })).toBeVisible();
  });

  test("a student is not shown the new course page", async ({ page }) => {
    await signIn(page, accounts.student, PASSWORD);
    await page.goto("/courses/new");
    await expect(page.getByRole("heading", { name: "We can't find that page" })).toBeVisible();
  });
});

test.describe("search", () => {
  test("a student finds published material and never a draft", async ({ page }) => {
    await signIn(page, accounts.student, PASSWORD);
    await page.getByRole("searchbox", { name: "Search" }).fill("merge");
    await page.getByRole("searchbox", { name: "Search" }).press("Enter");
    await expect(page).toHaveURL(/\/search\?q=merge/);
    await expect(page.getByRole("link", { name: /Merge sort implementation/ }).first()).toBeVisible();
    await expect(page.getByText("Draft", { exact: true })).toHaveCount(0);
    await page.getByRole("tab", { name: /^Lectures/ }).click();
    await expect(page.getByRole("link", { name: /Merge sort implementation/ })).toHaveCount(0);
  });

  test("a word that matches nothing says so", async ({ page }) => {
    await signIn(page, accounts.student, PASSWORD);
    await page.goto("/search?q=zzzzqqq");
    await expect(page.getByText("Nothing matches this search.")).toBeVisible();
  });
});

test.describe("new course", () => {
  test("a teacher creates a course with code, term and description", async ({ page }) => {
    await signIn(page, accounts.teacher, PASSWORD);
    await page.goto("/courses");
    await page.getByRole("link", { name: "New course" }).click();
    await expect(page).toHaveURL(/\/courses\/new/);
    await page.getByLabel("Course title").fill("Distributed Systems");
    await page.getByLabel("Code", { exact: true }).fill("CS 420");
    await page.getByLabel("Description").fill("Consensus, replication and failure.");
    await page.getByRole("button", { name: "Create course" }).click();
    await expect(page).toHaveURL(/\/course\/settings\?id=/);
    await expect(page.getByRole("heading", { name: "Distributed Systems" })).toBeVisible();
    // Follow links rather than reload: the mock keeps its data in the page.
    await page.getByRole("navigation", { name: "Main" }).first().getByRole("link", { name: "Courses" }).click();
    await expect(page.getByRole("link", { name: "Distributed Systems" })).toBeVisible();
    await expect(page.getByText("CS 420")).toBeVisible();
  });

  test("a new course is a draft that students cannot see until a teacher publishes it", async ({ page }) => {
    await signIn(page, accounts.teacher, PASSWORD);
    await page.goto("/courses/new");
    await page.getByLabel("Course title").fill("Operating Systems");
    await page.getByRole("button", { name: "Create course" }).click();
    await expect(page.getByRole("heading", { name: /Operating Systems/ })).toBeVisible();

    // The notice is on every screen of the course, with the way out.
    const notice = page.getByRole("status").filter({ hasText: "This course is a draft." });
    await expect(notice).toContainText("Students cannot see it yet");
    await page.getByRole("tab", { name: "People" }).click();
    await expect(notice).toBeVisible();

    await notice.getByRole("button", { name: "Publish course" }).click();
    const dialog = page.getByRole("dialog", { name: "Publish this course?" });
    await expect(dialog).toContainText("stay hidden from students");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(notice).toBeVisible();

    await notice.getByRole("button", { name: "Publish course" }).click();
    await page.getByRole("dialog", { name: "Publish this course?" }).getByRole("button", { name: "Publish", exact: true }).click();
    await expect(notice).toHaveCount(0);
    await expect(page.getByRole("heading", { name: /Operating Systems/ })).not.toContainText("Draft");
    // The course list no longer calls it a draft.
    await page.getByRole("navigation", { name: "Main" }).first().getByRole("link", { name: "Courses" }).click();
    await expect(page.getByRole("link", { name: "Operating Systems" })).toBeVisible();
    await expect(page.getByText(/, draft/i)).toHaveCount(0);
  });

  test("a course that is already published shows no draft notice", async ({ page }) => {
    await signIn(page, accounts.teacher, PASSWORD);
    await page.goto("/courses");
    await page.getByRole("link", { name: "Algorithms and Data Structures" }).first().click();
    await expect(page.getByRole("tablist")).toBeVisible();
    await expect(page.getByText("This course is a draft.")).toHaveCount(0);
  });

  test("an empty title is refused", async ({ page }) => {
    await signIn(page, accounts.teacher, PASSWORD);
    await page.goto("/courses/new");
    await expect(page.getByRole("button", { name: "Create course" })).toBeDisabled();
  });
});

test.describe("language", () => {
  test("the choice changes the interface, dates included, and is kept", async ({ page }) => {
    await page.goto("/login");
    await page.getByLabel("Language").selectOption("uk");
    await expect(page.getByRole("heading", { name: "Вхід" })).toBeVisible();
    await page.getByLabel("Електронна пошта").fill(accounts.student);
    await page.getByLabel("Пароль", { exact: true }).fill(PASSWORD);
    await page.getByRole("button", { name: "Увійти", exact: true }).click();
    await expect(page.getByRole("heading", { name: /Добр(ий день|ого ранку|ий вечір), Maya/ })).toBeVisible();
    await expect(page.getByRole("link", { name: "Курси" }).first()).toBeVisible();
    // Dates use Ukrainian month names.
    await expect(page.getByText(/жовт/).first()).toBeVisible();

    await page.reload();
    await expect(page.getByRole("link", { name: "Курси" }).first()).toBeVisible();

    await page.goto("/profile");
    await page.getByLabel("Мова").selectOption("en");
    await expect(page.getByRole("link", { name: "Courses" }).first()).toBeVisible();
  });
});
