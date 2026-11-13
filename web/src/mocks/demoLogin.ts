// The sign-in form under `pnpm dev:mock` starts filled in with the demo student. Without the mock flag
// next.config.ts points this import at noDemoLogin.ts, so a real build carries no sample address or password.
import { accounts, PASSWORD } from "./identities";

export const demo: { email: string; password: string } = { email: accounts.student, password: PASSWORD };
