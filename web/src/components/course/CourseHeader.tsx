"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useState, type ReactNode } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { StatusPill } from "@/components/StatusPill";
import { useCourse, useCourseInfo, usePublishCourse } from "@/lib/api/hooks/courses";
import { ownerName } from "@/lib/course";
import { plural } from "@/lib/format";
import { useT } from "@/lib/i18n";
import { useCourseAccess } from "./useCourseAccess";
import { courseHref } from "@/lib/routes";

type Tab = { id: string; label: string; href: string };

/** Breadcrumb, title, subtitle and the section tabs shared by every course screen. */
export function CourseHeader({ courseId, subtitle }: { courseId: string; subtitle?: ReactNode }) {
  const t = useT();
  const pathname = usePathname();
  const course = useCourse(courseId);
  const info = useCourseInfo(courseId);
  const { staff } = useCourseAccess(courseId);
  const publish = usePublishCourse(courseId);
  const [asking, setAsking] = useState(false);
  // A draft course is invisible to students, whoever is on its roster (ADR-0019): say so, where it can be fixed.
  const draft = staff && course.data?.status === "draft";
  const tabs: Tab[] = staff
    ? [
        { id: "content", label: t("course.content"), href: courseHref(courseId) },
        { id: "grades", label: t("course.gradebook"), href: courseHref(courseId, "grades") },
        { id: "people", label: t("course.people"), href: courseHref(courseId, "people") },
        { id: "settings", label: t("course.settings"), href: courseHref(courseId, "settings") },
      ]
    : [
        { id: "content", label: t("course.content"), href: courseHref(courseId) },
        { id: "grades", label: t("course.myGrades"), href: courseHref(courseId, "grades") },
        { id: "people", label: t("course.people"), href: courseHref(courseId, "people") },
      ];
  const current = pathname.startsWith("/course/grades") ? "grades" : pathname.startsWith("/course/people") ? "people" : pathname.startsWith("/course/settings") ? "settings" : "content";
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
      <h1 className="page-title" style={{ margin: "6px 0 4px" }}>
        {course.data?.title ?? " "}
        {draft && <>{" "}<StatusPill tone="plain">{t("common.draft")}</StatusPill></>}
      </h1>
      <p className="muted">{subtitle ?? defaultSub}</p>
      {draft && (
        <div role="status" className="alert alert-info" style={{ margin: "12px 0", flexWrap: "wrap", alignItems: "center" }}>
          <span style={{ flex: "1 1 320px" }}>{t("course.draftNote")}</span>
          <button type="button" className="btn btn-primary" onClick={() => { publish.reset(); setAsking(true); }}>{t("course.publish")}</button>
        </div>
      )}
      {asking && (
        <ConfirmDialog
          title={t("course.publishQ")}
          confirm={t("course.publishConfirm")}
          pending={publish.pending}
          error={publish.error?.detail ?? publish.error?.title}
          onClose={() => setAsking(false)}
          onConfirm={async () => { if (await publish.run()) setAsking(false); }}
        >
          {t("course.publishLead")}
        </ConfirmDialog>
      )}
      <div role="tablist" aria-label={t("course.sections")} className="tabs">
        {tabs.map((t) => (
          <Link key={t.id} href={t.href} role="tab" aria-selected={t.id === current}>{t.label}</Link>
        ))}
      </div>
    </>
  );
}
