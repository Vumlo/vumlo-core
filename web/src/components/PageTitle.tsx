"use client";

import { usePathname, useSearchParams } from "next/navigation";
import { useEffect } from "react";
import { useCourse, useItem } from "@/lib/api/hooks/courses";
import { appName } from "@/lib/appName";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";

/** Screens whose name does not depend on data. */
const SCREENS: Record<string, MessageKey> = {
  "/": "nav.home",
  "/courses": "nav.courses",
  "/courses/new": "title.newCourse",
  "/calendar": "nav.calendar",
  "/grading": "nav.grading",
  "/grades": "nav.grades",
  "/profile": "profile.title",
  "/search": "search.title",
  "/console": "console.title",
  "/console/login": "console.login.title",
  "/console/change-password": "change.title",
  "/console/people": "ap.people",
  "/console/groups": "ag.groups",
  "/console/terms": "ac.title",
  "/console/institution": "ai.title",
  "/console/audit": "au.title",
  "/login": "login.title",
  "/register": "register.title",
  "/verify-email": "verify.title",
  "/forgot-password": "forgot.title",
  "/reset-password": "reset.title",
  "/change-password": "change.title",
  "/download": "title.download",
};

/** What a course screen is, before the course's own name. */
const COURSE_SCREENS: Record<string, MessageKey> = {
  "/course/people": "title.people",
  "/course/settings": "title.settings",
  "/course/grades": "title.gradebook",
  "/course/grade": "nav.grading",
};

/**
 * The title of the browser tab: where you are first, so a narrow tab still tells screens apart, then the course, then the
 * app: "Lab report · CS 201 · Vumlo | Northfield". It follows the address, so no screen has to set it.
 */
export function PageTitle() {
  const t = useT();
  const path = usePathname().replace(/(.)\/$/, "$1");
  const params = useSearchParams();
  const inCourse = path.startsWith("/course");
  const courseId = inCourse ? (params.get("id") ?? "") : "";
  const itemId = path === "/course/item" || path === "/course/item/edit" ? (params.get("item") ?? "") : "";
  const course = useCourse(courseId || null).data?.title;
  const item = useItem(itemId || null).data?.title;

  const screen = SCREENS[path] ? t(SCREENS[path]) : COURSE_SCREENS[path] ? t(COURSE_SCREENS[path]) : undefined;
  const parts = [screen, path === "/course/item/edit" && item ? t("title.editItem", { name: item }) : item, course];
  const title = [...parts.filter(Boolean), appName].join(" · ");

  useEffect(() => {
    document.title = title;
  }, [title]);
  return null;
}
