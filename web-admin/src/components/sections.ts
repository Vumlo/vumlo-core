import type { IconName } from "@/components/Icon";
import type { MessageKey } from "@/lib/i18n/core";

/** The pages of the console, in the order of its sidebar: the path under `/`, its name and its icon. */
export const SECTIONS: [string, MessageKey, IconName][] = [
  ["people", "ap.people", "user"],
  ["groups", "ag.groups", "courses"],
  ["terms", "ac.title", "calendar"],
  ["units", "un.title", "courses"],
  ["plugins", "pl.title", "tune"],
  ["institution", "ai.title", "admin"],
  ["audit", "au.title", "file"],
];
