import type { Page } from "@playwright/test";
// The people of the mock world are named in one place (web/src/mocks/identities.ts).
import { accounts, PASSWORD } from "../../web/src/mocks/identities";

export { accounts, PASSWORD };

/** Puts a value in localStorage before the scripts of a page run. */
export async function store(page: Page, key: string, value: string) {
  await page.addInitScript(
    (entry) => {
      try {
        localStorage.setItem(entry.key, entry.value);
      } catch {
        // A frame without storage.
      }
    },
    { key, value },
  );
}
