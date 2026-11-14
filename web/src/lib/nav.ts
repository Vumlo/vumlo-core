import type { IconName } from "@/components/Icon";
import type { MessageKey } from "./i18n/core";
import type { Role } from "./session";

export type NavItem = { label: MessageKey; href: string; icon: IconName; match: (path: string) => boolean };

const starts = (prefix: string) => (path: string) => path === prefix || path.startsWith(`${prefix}/`);

const home: NavItem = { label: "nav.home", href: "/", icon: "home", match: (p) => p === "/" };
const courses: NavItem = { label: "nav.courses", href: "/courses", icon: "courses", match: (p) => starts("/courses")(p) || starts("/course")(p) };
const calendar: NavItem = { label: "nav.calendar", href: "/calendar", icon: "calendar", match: starts("/calendar") };

/** Navigation only hides links; the server decides what a role may do. */
const unitsItem: NavItem = { label: "nav.units", href: "/units", icon: "units", match: starts("/units") };

/** `units`: the person holds a role on at least one unit, whatever their account role (ADR-0094). */
export function navFor(role: Role, { units }: { units: boolean }): NavItem[] {
  const items = (): NavItem[] => {
    switch (role) {
      case "teacher":
        return [home, courses, calendar, { label: "nav.grading", href: "/grading", icon: "grading", match: starts("/grading") }];
      case "admin":
        // Administration is the console's (ADR-0095): its link is in the account menu.
        return [home, courses, calendar];
      default:
        return [home, courses, calendar, { label: "nav.grades", href: "/grades", icon: "grades", match: starts("/grades") }];
    }
  };
  return units ? [...items(), unitsItem] : items();
}
