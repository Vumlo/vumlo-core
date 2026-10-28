"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { CourseOutline } from "@/components/course/CourseOutline";
import { useContentTree, useCourse, useCourseInfo, useItemMeta } from "@/lib/api/hooks/courses";
import { findItem } from "@/lib/course";
import { useT } from "@/lib/i18n";
import s from "./Item.module.css";
import { courseHref, editItemHref } from "@/lib/routes";
import { usePluginTabs } from "@/lib/api/hooks/plugins";
import { useItemTab } from "@/lib/useRoute";
import { PluginFrame } from "./PluginFrame";
import { PluginTabs } from "./PluginTabs";

/**
 * Outline on the left, the item on the right. `focus` hides the outline and the plugin tabs (focus mode of the answer
 * editor); `keepOutline` keeps the outline in a focused page, so a quiz still shows the course around it.
 */
export function ItemFrame({ courseId, itemId, children, focus = false, keepOutline = false }: { courseId: string; itemId: string; children: (ctx: { crumbs: ReactNode }) => ReactNode; focus?: boolean; keepOutline?: boolean }) {
  const t = useT();
  const tree = useContentTree(courseId);
  const meta = useItemMeta(courseId);
  const info = useCourseInfo(courseId);
  const course = useCourse(courseId);
  const found = tree.data ? findItem(tree.data, itemId) : undefined;
  // The pages that plugins offer in the tab of an assignment (ADR-0066). None: the page is as it was.
  const pluginSlots = usePluginTabs();
  const tab = useItemTab();
  const tabs = found?.item.kind === "assignment" && !focus ? pluginSlots : [];
  const openTab = tabs.find((slot) => slot.plugin_id === tab);
  const crumbs = (
    <nav aria-label={t("common.breadcrumb")} className="crumbs">
      <Link href={courseHref(courseId)}>{info.data?.code ?? t("search.kind.course")}</Link>
      <span aria-hidden="true" style={{ padding: "0 8px" }}>/</span>
      <Link href={`${courseHref(courseId)}#sec-${found?.module.id ?? ""}`}>{found?.module.title ?? "…"}</Link>
    </nav>
  );
  return (
    <div className="container" style={{ paddingTop: 24 }}>
      <div className={focus && !keepOutline ? s.frameFocus : s.frame}>
        {(!focus || keepOutline) && tree.data && <CourseOutline courseId={courseId} tree={tree.data} meta={meta.data?.items ?? {}} code={info.data?.code} title={course.data?.title} currentItemId={itemId} />}
        <div className={s.main} style={focus ? { maxWidth: "none" } : undefined}>
          {tabs.length === 0 ? (
            children({ crumbs })
          ) : (
            <>
              <PluginTabs courseId={courseId} itemId={itemId} slots={tabs} current={openTab?.plugin_id ?? ""} />
              {openTab && <h2 className={s.pluginHeading}>{openTab.title}</h2>}
              {openTab && <PluginFrame key={openTab.plugin_id} assignmentId={itemId} slot={openTab} />}
              {/* Kept mounted while a plugin tab is open, so a student's unsent draft is not lost. */}
              <div hidden={openTab !== undefined}>{children({ crumbs })}</div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

export function PreviewBanner({ courseId, itemId }: { courseId: string; itemId: string }) {
  const t = useT();
  return (
    <div role="status" className={`alert alert-info ${s.preview}`}>
      <span style={{ flex: "1 1 auto" }}>{t("preview.banner")}</span>
      <Link href={editItemHref(courseId, itemId)} className="btn">{t("common.edit")}</Link>
    </div>
  );
}
