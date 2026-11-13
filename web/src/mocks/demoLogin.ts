// The sign-in form under `pnpm dev:mock` starts filled in with the demo student. Without the mock flag
// next.config.ts points this import at noDemoLogin.ts, so a real build carries no sample address or password.
import { accounts, PASSWORD } from "./identities";

export const demo: { email: string; password: string } = { email: accounts.student, password: PASSWORD };

/** The console's sign-in form starts filled in with the demo administrator, under the mock only. */
export const consoleDemo: { email: string; password: string } = { email: accounts.admin, password: PASSWORD };
