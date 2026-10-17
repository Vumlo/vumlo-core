import type { IconName } from "@/components/Icon";
import type { Role } from "./session";

export type NavItem = { label: string; href: string; icon: IconName; match: (path: string) => boolean };

const starts = (prefix: string) => (path: string) => path === prefix || path.startsWith(`${prefix}/`);

const home: NavItem = { label: "Home", href: "/", icon: "home", match: (p) => p === "/" };
const courses: NavItem = { label: "Courses", href: "/courses", icon: "courses", match: starts("/courses") };
const calendar: NavItem = { label: "Calendar", href: "/calendar", icon: "calendar", match: starts("/calendar") };

/** Navigation only hides links; the server decides what a role may do. */
export function navFor(role: Role): NavItem[] {
  switch (role) {
    case "teacher":
      return [home, courses, calendar, { label: "Grading", href: "/grading", icon: "grading", match: starts("/grading") }];
    case "admin":
      return [home, courses, calendar, { label: "Admin", href: "/admin", icon: "admin", match: starts("/admin") }];
    default:
      return [home, courses, calendar, { label: "Grades", href: "/grades", icon: "grades", match: starts("/grades") }];
  }
}
