"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { CourseOutline } from "@/components/course/CourseOutline";
import { useContentTree, useCourse, useCourseInfo, useItemMeta } from "@/lib/api/hooks/courses";
import { findItem } from "@/lib/course";
import { useT } from "@/lib/i18n";
import s from "./Item.module.css";

/** Outline on the left, the item on the right. `focus` hides the outline (focus mode of the answer editor). */
export function ItemFrame({ courseId, itemId, children, focus = false }: { courseId: string; itemId: string; children: (ctx: { crumbs: ReactNode }) => ReactNode; focus?: boolean }) {
  const t = useT();
  const tree = useContentTree(courseId);
  const meta = useItemMeta(courseId);
  const info = useCourseInfo(courseId);
  const course = useCourse(courseId);
  const found = tree.data ? findItem(tree.data, itemId) : undefined;
  const crumbs = (
    <nav aria-label={t("common.breadcrumb")} className="crumbs">
      <Link href={`/courses/${courseId}`}>{info.data?.code ?? t("search.kind.course")}</Link>
      <span aria-hidden="true" style={{ padding: "0 6px" }}>/</span>
      <Link href={`/courses/${courseId}#sec-${found?.module.id ?? ""}`}>{found?.module.title ?? "…"}</Link>
    </nav>
  );
  return (
    <div className="container" style={{ paddingTop: 24 }}>
      <div className={focus ? s.frameFocus : s.frame}>
        {!focus && tree.data && <CourseOutline courseId={courseId} tree={tree.data} meta={meta.data?.items ?? {}} code={info.data?.code} title={course.data?.title} currentItemId={itemId} />}
        <div className={s.main} style={focus ? { maxWidth: "none" } : undefined}>{children({ crumbs })}</div>
      </div>
    </div>
  );
}

export function PreviewBanner({ courseId, itemId }: { courseId: string; itemId: string }) {
  const t = useT();
  return (
    <div role="status" className={`alert alert-info ${s.preview}`}>
      <span style={{ flex: "1 1 auto" }}>{t("preview.banner")}</span>
      <Link href={`/courses/${courseId}/items/${itemId}/edit`} className="btn">{t("common.edit")}</Link>
    </div>
  );
}
