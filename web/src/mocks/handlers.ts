import { adminHandlers } from "./handlers/admin";
import { authHandlers } from "./handlers/auth";
import { coreHandlers } from "./handlers/core";
import { fileHandlers } from "./handlers/files";
import { pluginReportHandlers } from "./handlers/pluginReports";
import { unitHandlers } from "./handlers/units";
import { workHandlers } from "./handlers/work";

export const handlers = [...authHandlers, ...coreHandlers, ...workHandlers, ...adminHandlers, ...fileHandlers, ...pluginReportHandlers, ...unitHandlers];
