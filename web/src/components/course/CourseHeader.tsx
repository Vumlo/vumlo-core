"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";
import { useCourse, useCourseInfo } from "@/lib/api/hooks/courses";
import { ownerName } from "@/lib/course";
import { plural } from "@/lib/format";
import { useT } from "@/lib/i18n";
import { useCourseAccess } from "./useCourseAccess";

type Tab = { id: string; label: string; href: string };

/** Breadcrumb, title, subtitle and the section tabs shared by every course screen. */
export function CourseHeader({ courseId, subtitle }: { courseId: string; subtitle?: ReactNode }) {
  const t = useT();
  const pathname = usePathname();
  const course = useCourse(courseId);
  const info = useCourseInfo(courseId);
  const { staff } = useCourseAccess(courseId);
  const base = `/courses/${courseId}`;
  const tabs: Tab[] = staff
    ? [
        { id: "content", label: t("course.content"), href: base },
        { id: "grades", label: t("course.gradebook"), href: `${base}/grades` },
        { id: "people", label: t("course.people"), href: `${base}/people` },
        { id: "settings", label: t("course.settings"), href: `${base}/settings` },
      ]
    : [
        { id: "content", label: t("course.content"), href: base },
        { id: "grades", label: t("course.myGrades"), href: `${base}/grades` },
        { id: "people", label: t("course.people"), href: `${base}/people` },
      ];
  const current = pathname.startsWith(`${base}/grades`) ? "grades" : pathname.startsWith(`${base}/people`) ? "people" : pathname.startsWith(`${base}/settings`) ? "settings" : "content";
  const i = info.data;
  const defaultSub = !i
    ? ""
    : staff
      ? `${i.term_name ?? t("courses.noTerm")}, ${plural(i.student_count, "student")}${i.former_count ? ` ${t("course.and")} ${plural(i.former_count, "former student")}` : ""}`
      : `${i.term_name ?? ""}${i.term_name ? ", " : ""}${t("course.taughtBy", { name: ownerName(i) ?? "" })}`;

  return (
    <>
      <nav aria-label={t("common.breadcrumb")} className="crumbs">
        <Link href="/courses">{t("nav.courses")}</Link>
        <span aria-hidden="true" style={{ padding: "0 6px" }}>/</span>
        <span aria-current="page">{i?.code || course.data?.title || "…"}</span>
      </nav>
      <h1 className="page-title" style={{ margin: "6px 0 4px" }}>{course.data?.title ?? " "}</h1>
      <p className="muted">{subtitle ?? defaultSub}</p>
      <div role="tablist" aria-label={t("course.sections")} className="tabs">
        {tabs.map((t) => (
          <Link key={t.id} href={t.href} role="tab" aria-selected={t.id === current}>{t.label}</Link>
        ))}
      </div>
    </>
  );
}
