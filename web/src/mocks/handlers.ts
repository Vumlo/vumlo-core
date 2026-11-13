import { adminHandlers } from "./handlers/admin";
import { authHandlers } from "./handlers/auth";
import { consoleHandlers } from "./handlers/console";
import { coreHandlers } from "./handlers/core";
import { fileHandlers } from "./handlers/files";
import { pluginReportHandlers } from "./handlers/pluginReports";
import { workHandlers } from "./handlers/work";

export const handlers = [...authHandlers, ...consoleHandlers, ...coreHandlers, ...workHandlers, ...adminHandlers, ...fileHandlers, ...pluginReportHandlers];
