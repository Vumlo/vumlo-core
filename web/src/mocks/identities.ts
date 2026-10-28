// Who exists in the mock world, in one place. It imports nothing, so the Playwright suite (e2e/routes.ts) can read it too.
// Nothing outside src/mocks/ and e2e/ may name these accounts: the real app has no demo people.

/** The institution's mail domain in the mock; sign-up and Google sign-in only take addresses ending in it. */
export const DOMAIN = "northfield.edu";

export const PASSWORD = "correct horse";
export const TEMP_PASSWORD = "temporary password";

/** The three people the suite and `pnpm dev:mock` sign in as. The seed creates them with these names. */
export const accounts = {
  student: `maya.okafor@${DOMAIN}`,
  teacher: `elena.voss@${DOMAIN}`,
  admin: `marta.kovalska@${DOMAIN}`,
} as const;
