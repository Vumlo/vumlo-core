import { setupWorker } from "msw/browser";
import { getDb, setDb } from "@/mocks/db";
import { accounts, PASSWORD } from "@/mocks/identities";
import { handlers } from "@/mocks/handlers";
import { runAsConsole } from "@/mocks/logic";
import { createSeed } from "@/mocks/seed";
import { consoleHandlers } from "./console";
import { consolePluginHandlers } from "./consolePlugins";
import { mailHandlers } from "./mail";

/** Starts the mock API under `npm run dev:mock`: the student app's world plus what only the console asks (ADR-0095). */
export async function start() {
  setDb(createSeed());
  // In the console an admin holds every ability on every unit; and Elena Voss manages Computer Science there.
  runAsConsole();
  const db = getDb();
  const elena = db.users.find((u) => u.role === "teacher" && u.surname === "Voss")!;
  (db.unitRoles["unit-cs"] ??= {})[elena.id] = "manager";
  // The sign-in form starts filled in with the demo administrator.
  Object.assign(window, { __consoleDemo: { email: accounts.admin, password: PASSWORD } });
  await setupWorker(...consoleHandlers, ...consolePluginHandlers, ...mailHandlers, ...handlers).start({ onUnhandledFrame: "bypass", quiet: true });
}
