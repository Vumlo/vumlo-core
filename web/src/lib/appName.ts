import { clientConfig } from "@/lib/clientConfig";

/** The name of the app: "Vumlo | Northfield", or just "Vumlo" when the short name is Vumlo itself. */
export function appNameOf(shortName: string): string {
  const short = shortName.trim();
  return short.toLowerCase() === "vumlo" ? "Vumlo" : `Vumlo | ${short}`;
}

/** From client-config.json, so it is the same in the page's own HTML and after it has loaded. */
export const appName = appNameOf(clientConfig.public.institution.short_name);
