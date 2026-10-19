"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { BlockRenderer, plainText } from "@/components/doc/BlockRenderer";
import { Icon } from "@/components/Icon";
import { useContentTree, useCourseInfo, useItemMeta } from "@/lib/api/hooks/courses";
import { allItems } from "@/lib/course";
import { dayLong, shortDay } from "@/lib/format";
import { bodyOf, type Item } from "@/lib/api/types";
import { ItemFrame, PreviewBanner } from "./ItemFrame";
import { useT } from "@/lib/i18n";
import s from "./Item.module.css";
import { itemHref } from "@/lib/routes";

export function LecturePage({ item, courseId, staff }: { item: Item; courseId: string; staff: boolean }) {
  const t = useT();
  const tree = useContentTree(courseId);
  const meta = useItemMeta(courseId);
  const info = useCourseInfo(courseId);
  const doc = useMemo(() => bodyOf(item), [item]);
  const headings = doc.blocks.filter((b) => b.type === "heading" && b.props.level <= 2).map((b) => ({ id: b.id, label: b.type === "heading" ? (b.content ?? []).map((r) => (r.type === "text" ? r.text : "")).join("") : "" }));
  const [current, setCurrent] = useState<string | null>(null);
  const minutes = meta.data?.items[item.id]?.read_minutes ?? Math.max(1, Math.round(plainText(doc).split(/\s+/).filter(Boolean).length / 200));

  useEffect(() => {
    if (headings.length === 0) return;
    const els = headings.map((h) => document.getElementById(h.id)).filter((e): e is HTMLElement => !!e);
    const io = new IntersectionObserver((entries) => {
      const visible = entries.filter((e) => e.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0];
      if (visible) setCurrent(visible.target.id);
    }, { rootMargin: "-80px 0px -60% 0px" });
    els.forEach((e) => io.observe(e));
    return () => io.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item.id, doc.blocks.length]);

  const items = tree.data ? allItems(tree.data) : [];
  const at = items.findIndex((i) => i.id === item.id);
  const prevLecture = items.slice(0, Math.max(0, at)).reverse().find((i) => i.kind === "text");
  const next = at >= 0 ? items[at + 1] : undefined;

  return (
    <ItemFrame courseId={courseId} itemId={item.id}>
      {({ crumbs }) => (
        <div className={s.lecture}>
          <article className={s.article}>
            {staff && <PreviewBanner courseId={courseId} itemId={item.id} />}
            {crumbs}
            <div className={s.kind}>
              <span aria-hidden="true" className={s.kindIcon} data-kind="lecture"><Icon name="lecture" size={15} strokeWidth={2} /></span>
              <span className={s.kindLabel} data-kind="lecture">{t("row.lecture")}</span>
              <span className="muted" style={{ fontSize: 14 }}>{t("row.readMinutes", { n: minutes })}</span>
            </div>
            <h1 className={s.lectureTitle}>{item.title}</h1>
            <p className={s.byline}>{info.data?.owner_name}, {t("lecture.updated", { day: dayLong(item.updated_at) })}</p>
            <BlockRenderer doc={doc} />
            <nav aria-label={t("lecture.pager")} className={s.pager}>
              {prevLecture ? (
                <Link href={itemHref(courseId, prevLecture.id)} className={s.pagerLink}>
                  <span className={s.pagerSmall}>{t("lecture.prev")}</span>
                  <span style={{ fontSize: 15, fontWeight: 600 }}>{prevLecture.title}</span>
                </Link>
              ) : null}
              {next && (
                <Link href={itemHref(courseId, next.id)} className={s.pagerLink} data-kind={next.kind === "assignment" ? "assignment" : "lecture"} style={{ textAlign: "right" }}>
                  <span className={s.pagerSmall}>
                    {next.kind === "assignment" ? (next.assignment?.due_at ? t("lecture.nextAssignmentDue", { day: shortDay(next.assignment.due_at) }) : t("lecture.nextAssignment")) : t("lecture.next")}
                  </span>
                  <span style={{ fontSize: 15, fontWeight: 600 }}>{next.title}</span>
                </Link>
              )}
            </nav>
          </article>
          {headings.length > 0 && (
            <aside aria-labelledby="toc-h" className={s.toc}>
              <h2 id="toc-h" style={{ margin: "0 0 8px", fontSize: 13, fontWeight: 600, color: "var(--muted)" }}>{t("lecture.onThisPage")}</h2>
              <nav className={s.tocNav}>
                {headings.map((h) => (
                  <a key={h.id} href={`#${h.id}`} className={s.tocLink} aria-current={(current ?? headings[0]!.id) === h.id ? "location" : undefined}>{h.label}</a>
                ))}
              </nav>
            </aside>
          )}
        </div>
      )}
    </ItemFrame>
  );
}
