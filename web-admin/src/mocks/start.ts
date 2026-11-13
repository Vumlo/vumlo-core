import { setupWorker } from "msw/browser";
import { setDb } from "@/mocks/db";
import { accounts, PASSWORD } from "@/mocks/identities";
import { handlers } from "@/mocks/handlers";
import { createSeed } from "@/mocks/seed";
import { consoleHandlers } from "./console";
import { consolePluginHandlers } from "./consolePlugins";
import { seedUnitRoles, unitHandlers } from "./units";

/** Starts the mock API under `npm run dev:mock`: the student app's world plus what only the console asks (ADR-0095). */
export async function start() {
  setDb(createSeed());
  seedUnitRoles();
  // The sign-in form starts filled in with the demo administrator.
  Object.assign(window, { __consoleDemo: { email: accounts.admin, password: PASSWORD } });
  await setupWorker(...consoleHandlers, ...consolePluginHandlers, ...unitHandlers, ...handlers).start({ onUnhandledFrame: "bypass", quiet: true });
}
