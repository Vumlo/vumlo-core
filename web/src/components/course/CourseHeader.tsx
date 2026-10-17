"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";
import { useCourse, useCourseInfo } from "@/lib/api/hooks/courses";
import { plural } from "@/lib/format";
import { useCourseAccess } from "./useCourseAccess";

type Tab = { id: string; label: string; href: string };

/** Breadcrumb, title, subtitle and the section tabs shared by every course screen. */
export function CourseHeader({ courseId, subtitle }: { courseId: string; subtitle?: ReactNode }) {
  const pathname = usePathname();
  const course = useCourse(courseId);
  const info = useCourseInfo(courseId);
  const { staff } = useCourseAccess(courseId);
  const base = `/courses/${courseId}`;
  const tabs: Tab[] = staff
    ? [
        { id: "content", label: "Content", href: base },
        { id: "grades", label: "Gradebook", href: `${base}/grades` },
        { id: "people", label: "People", href: `${base}/people` },
        { id: "settings", label: "Settings", href: `${base}/settings` },
      ]
    : [
        { id: "content", label: "Content", href: base },
        { id: "grades", label: "My grades", href: `${base}/grades` },
        { id: "people", label: "People", href: `${base}/people` },
      ];
  const current = pathname.startsWith(`${base}/grades`) ? "grades" : pathname.startsWith(`${base}/people`) ? "people" : pathname.startsWith(`${base}/settings`) ? "settings" : "content";
  const i = info.data;
  const defaultSub = !i
    ? ""
    : staff
      ? `${i.term_name ?? "No term"}, ${plural(i.student_count, "student")}${i.former_count ? ` and ${plural(i.former_count, "former student")}` : ""}`
      : `${i.term_name ?? ""}${i.term_name ? ", " : ""}taught by ${i.owner_name}`;

  return (
    <>
      <nav aria-label="Breadcrumb" className="crumbs">
        <Link href="/courses">Courses</Link>
        <span aria-hidden="true" style={{ padding: "0 6px" }}>/</span>
        <span aria-current="page">{i?.code || course.data?.title || "…"}</span>
      </nav>
      <h1 className="page-title" style={{ margin: "6px 0 4px" }}>{course.data?.title ?? " "}</h1>
      <p className="muted">{subtitle ?? defaultSub}</p>
      <div role="tablist" aria-label="Course sections" className="tabs">
        {tabs.map((t) => (
          <Link key={t.id} href={t.href} role="tab" aria-selected={t.id === current}>{t.label}</Link>
        ))}
      </div>
    </>
  );
}
