"use client";

import Link from "next/link";
import { useState } from "react";
import { Icon } from "@/components/Icon";
import type { ItemMeta } from "@/lib/api/types";
import type { ContentTree } from "@/lib/api/types";
import { isAssignment, kindOf, outlineMark, sections } from "@/lib/course";
import { useT } from "@/lib/i18n";
import s from "./CourseOutline.module.css";
import { courseHref, itemHref } from "@/lib/routes";

type Props = {
  courseId: string;
  tree: ContentTree;
  meta: Record<string, ItemMeta>;
  code?: string;
  title?: string;
  currentItemId?: string;
};

/** The left column of course pages: sections with their progress and every item. Items use the lecture/assignment icon colours. */
export function CourseOutline({ courseId, tree, meta, code, title, currentItemId }: Props) {
  const t = useT();
  const secs = sections(tree);
  const hasCurrent = (ids: { id: string }[]) => ids.some((i) => i.id === currentItemId);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const isOpen = (id: string, items: { id: string }[]) => open[id] ?? (currentItemId ? hasCurrent(items) : true);

  return (
    <nav aria-label={t("outline.label")} className={s.nav}>
      {title ? (
        <Link href={courseHref(courseId)} className={s.course}>
          <span className={s.courseTitle}>{title}</span>
          <span className={s.courseCode}>{code}</span>
        </Link>
      ) : (
        <p className={s.label}>{t("outline.label")}</p>
      )}
      <div className={s.list}>
        {secs.map((sec) => {
          const asg = sec.items.filter(isAssignment);
          const done = asg.filter((i) => ["graded", "submitted"].includes(meta[i.id]?.status ?? "")).length;
          const expanded = isOpen(sec.id, sec.items);
          return (
            <div key={sec.id}>
              <button type="button" className={s.sectionBtn} aria-expanded={expanded} onClick={() => setOpen((o) => ({ ...o, [sec.id]: !expanded }))}>
                <Icon name={expanded ? "chevronDown" : "chevronRight"} size={16} style={{ color: "var(--muted)", flex: "none" }} strokeWidth={2} />
                <span className={s.sectionTitle}>{sec.title}</span>
                {asg.length > 0 && <span className={s.progress}>{done}/{asg.length}</span>}
              </button>
              {expanded &&
                sec.items.map((i) => {
                  const k = kindOf(i);
                  const mark = k === "assignment" ? outlineMark(meta[i.id]) : null;
                  return (
                    <Link
                      key={i.id}
                      href={itemHref(courseId, i.id)}
                      title={i.title}
                      className={s.item}
                      aria-current={i.id === currentItemId ? "page" : undefined}
                    >
                      <Icon name={k} size={15} strokeWidth={2} className={k === "lecture" ? s.iconLecture : s.iconAssignment} />
                      <span className={s.itemTitle}>{i.title}</span>
                      {mark && <span className={s.mark} data-tone={mark.tone}>{mark.text}</span>}
                    </Link>
                  );
                })}
            </div>
          );
        })}
      </div>
    </nav>
  );
}
