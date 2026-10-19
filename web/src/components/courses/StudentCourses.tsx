"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useMemo, useState } from "react";
import { Icon } from "@/components/Icon";
import { ItemTile } from "@/components/ItemTile";
import { useCourseCards } from "@/lib/api/hooks/courses";
import { shortDay } from "@/lib/format";
import { useT } from "@/lib/i18n";
import s from "@/components/Courses.module.css";
import { courseHref } from "@/lib/routes";

export function StudentCourses() {
  const t = useT();
  const params = useSearchParams();
  const cards = useCourseCards();
  const [tab, setTab] = useState<"current" | "past">("current");
  const [q, setQ] = useState(params.get("q") ?? "");

  const { current, past } = useMemo(() => {
    const all = (cards.data ?? []).filter((c) => `${c.code} ${c.course.title}`.toLowerCase().includes(q.trim().toLowerCase()));
    const cur = all.filter((c) => !c.term_ended && c.course.status !== "archived");
    const old = all.filter((c) => !cur.includes(c));
    const terms = [...new Set(old.map((c) => c.term_name ?? t("courses.earlier")))];
    return { current: cur, past: terms.map((name) => ({ term: name, items: old.filter((c) => (c.term_name ?? t("courses.earlier")) === name) })) };
  }, [cards.data, q, t]);
  const pastCount = past.reduce((a, g) => a + g.items.length, 0);
  const term = current[0]?.term_name;

  return (
    <div className="container">
      <div className="page-head" style={{ alignItems: "center" }}>
        <h1 className="page-title" style={{ flex: "1 1 auto" }}>{t("nav.courses")}</h1>
        <label className="search" style={{ flex: "0 1 240px", background: "var(--surface)" }}>
          <Icon name="search" size={16} />
          <input type="search" aria-label={t("courses.filter")} placeholder={t("courses.filterPlaceholder")} value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
        <div role="tablist" aria-label={t("courses.which")} className="segmented">
          <button type="button" role="tab" aria-selected={tab === "current"} onClick={() => setTab("current")}>{t("courses.current")}, {current.length}</button>
          <button type="button" role="tab" aria-selected={tab === "past"} onClick={() => setTab("past")}>{t("courses.past")}, {pastCount}</button>
        </div>
      </div>

      {!cards.data ? <p className="loading" role="status">{t("common.loading")}</p> : tab === "current" ? (
        <section aria-labelledby="term-h">
          <h2 id="term-h" className="section-title">{term ?? t("courses.thisTerm")}</h2>
          {current.length === 0 ? <p className="empty">{q ? t("courses.noneMatch") : t("courses.noneCurrent")}</p> : (
            <div className={s.grid}>
              {current.map((c) => (
                <Link key={c.course.id} href={courseHref(c.course.id)} className={`card ${s.card}`}>
                  <span className={s.code}>{c.code}</span>
                  <span className={s.title}>{c.course.title}</span>
                  <span className="muted" style={{ fontSize: 14 }}>{c.owner_name}</span>
                  {c.progress && (
                    <span className={s.progress}>
                      <span className="row" style={{ justifyContent: "space-between", gap: 12, fontSize: 14 }}>
                        <span>{t("courses.submitted", { done: c.progress.submitted, total: c.progress.total })}</span>
                        <span className="muted">{t("courses.graded", { n: c.progress.graded })}</span>
                      </span>
                      <span role="meter" aria-label={t("courses.submittedLabel")} aria-valuemin={0} aria-valuemax={c.progress.total} aria-valuenow={c.progress.submitted} className="meter" style={{ marginTop: 8 }}>
                        <span style={{ width: `${c.progress.total ? Math.round((c.progress.submitted / c.progress.total) * 100) : 0}%` }} />
                      </span>
                    </span>
                  )}
                  <span className={s.next}>
                    <ItemTile kind="assignment" small />
                    <span className="ellipsis">{c.next ? t("courses.next", { title: c.next.title, when: shortDay(c.next.due_at) }) : t("courses.nothingDue")}</span>
                  </span>
                </Link>
              ))}
            </div>
          )}
        </section>
      ) : (
        <>
          {past.length === 0 && <p className="empty">{t("courses.nonePast")}</p>}
          {past.map((g) => (
            <section key={g.term} style={{ marginBottom: 28 }}>
              <h2 className="section-title">{g.term}</h2>
              <div className="card card-list">
                {g.items.map((c) => (
                  <Link key={c.course.id} href={courseHref(c.course.id)} className={s.pastRow}>
                    <span className={s.pastCode}>{c.code}</span>
                    <span className="grow">
                      <span style={{ display: "block", fontWeight: 500 }}>{c.course.title}</span>
                      <span className="muted" style={{ display: "block", fontSize: 13 }}>{c.owner_name}</span>
                    </span>
                    <span className="muted" style={{ flex: "none", fontSize: 13 }}>{t("courses.readOnly")}</span>
                    <Icon name="chevronRight" size={16} strokeWidth={2} style={{ color: "var(--muted)", flex: "none" }} />
                  </Link>
                ))}
              </div>
            </section>
          ))}
          <p className="muted" style={{ margin: "0 2px", fontSize: 14 }}>{t("courses.pastNote")}</p>
        </>
      )}
    </div>
  );
}
