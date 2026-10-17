import { setupWorker } from "msw/browser";
import { createSeed } from "./seed";
import { setDb } from "./db";
import { handlers } from "./handlers";

setDb(createSeed());

export const worker = setupWorker(...handlers);
