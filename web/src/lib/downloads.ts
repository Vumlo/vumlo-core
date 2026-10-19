"use client";

import { useSyncExternalStore } from "react";
import { isDesktop } from "./api/client";
import { useInstitution } from "./api/hooks/institution";
import type { MessageKey } from "./i18n/core";
import type { AppDownload, AppPlatform } from "./api/types";

// The links to the institution's desktop app (ADR-0064): an admin sets them, the public profile lists them,
// and with at least one the web app shows a hint on the home page and a download page.

export type System = "windows" | "macos" | "linux";

/** The installers by system, in the order the page lists them. */
export const SYSTEMS: { id: System; label: MessageKey; platforms: { platform: AppPlatform; label: MessageKey }[] }[] = [
  { id: "windows", label: "app.system.windows", platforms: [{ platform: "windows", label: "app.platform.windows" }] },
  {
    id: "macos",
    label: "app.system.macos",
    platforms: [
      { platform: "macos_arm", label: "app.platform.macos_arm" },
      { platform: "macos_intel", label: "app.platform.macos_intel" },
    ],
  },
  {
    id: "linux",
    label: "app.system.linux",
    platforms: [
      { platform: "linux_deb", label: "app.platform.linux_deb" },
      { platform: "linux_appimage", label: "app.platform.linux_appimage" },
    ],
  },
];

/** The desktop system this browser runs on, or null on a phone, a tablet or a system the app is not made for. */
export function detectSystem(): System | null {
  if (typeof navigator === "undefined") return null;
  const nav = navigator as Navigator & { userAgentData?: { platform?: string; mobile?: boolean } };
  const agent = nav.userAgent;
  if (nav.userAgentData?.mobile || /Android|iPhone|iPad|iPod|CrOS/i.test(agent)) return null;
  const platform = `${nav.userAgentData?.platform ?? nav.platform ?? ""} ${agent}`;
  // An iPad asks for the desktop site and says it is a Mac; a Mac has no touch screen.
  if (/Mac/i.test(platform)) return nav.maxTouchPoints > 1 ? null : "macos";
  if (/Win/i.test(platform)) return "windows";
  if (/Linux|X11/i.test(platform)) return "linux";
  return null;
}

const never = () => () => undefined;

/** The system of this browser; null before the page is in the browser. */
export function useSystem(): System | null {
  return useSyncExternalStore(never, detectSystem, () => null);
}

/** The links an institution offers, and the one this computer needs if there is exactly one for its system. */
export function useAppOffer() {
  const institution = useInstitution();
  const system = useSystem();
  // The app does not offer itself to itself.
  const links: AppDownload[] = isDesktop() ? [] : (institution.data?.app_downloads ?? []);
  const mine = SYSTEMS.find((s) => s.id === system);
  const forSystem = links.filter((link) => mine?.platforms.some((p) => p.platform === link.platform));
  return { links, system, loaded: !!institution.data, direct: forSystem.length === 1 ? forSystem[0] : undefined, hasForSystem: forSystem.length > 0 };
}
