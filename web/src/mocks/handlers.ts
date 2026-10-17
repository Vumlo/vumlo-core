import { adminHandlers } from "./handlers/admin";
import { coreHandlers } from "./handlers/core";
import { workHandlers } from "./handlers/work";

export const handlers = [...coreHandlers, ...workHandlers, ...adminHandlers];
