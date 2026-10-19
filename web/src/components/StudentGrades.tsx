"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { ItemTile } from "@/components/ItemTile";
import { StatusPill } from "@/components/StatusPill";
import { useMyWork } from "@/lib/api/hooks/work";
import type { MyWorkItem } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { dueShort, statusPill } from "@/lib/course";
import { sumPoints } from "@/lib/points";
import { useT } from "@/lib/i18n";
import s from "./Grades.module.css";
import { courseHref, itemHref } from "@/lib/routes";

/** Grades across all courses. Only published grades carry a score; everything else shows its state. */
export function StudentGrades() {
  const t = useT();
  const work = useMyWork();
  const [filter, setFilter] = useState<"all" | "graded">("all");
  const terms = useMemo(() => [...new Set((work.data?.courses ?? []).map((c) => c.term_name ?? t("courses.noTerm")))].sort().reverse(), [work.data, t]);
  const [term, setTerm] = useState<string>("");
  const activeTerm = term || terms[0] || "";
  const today = now();

  const courses = (work.data?.courses ?? [])
    .filter((c) => (c.term_name ?? t("courses.noTerm")) === activeTerm)
    .map((c) => {
      const rows = (work.data?.items ?? []).filter((i) => i.course_id === c.id && i.type !== "form");
      const graded = rows.filter((r) => r.status === "graded");
      return { c, rows: rows.filter((r) => filter === "all" || r.status === "graded"), total: rows.length, graded };
    });

  return (
    <div className="container">
      <div className="page-head" style={{ alignItems: "center" }}>
        <div style={{ flex: "1 1 auto" }}>
          <h1 className="page-title">{t("nav.grades")}</h1>
          <p className="page-sub">{activeTerm}. {t("grades.lead")}</p>
        </div>
        <select aria-label={t("newCourse.term")} className="select" style={{ width: "auto", minHeight: 40 }} value={activeTerm} onChange={(e) => setTerm(e.target.value)}>
          {terms.map((name) => <option key={name}>{name}</option>)}
        </select>
        <div role="radiogroup" aria-label={t("grades.show")} className="segmented">
          <button type="button" role="radio" aria-checked={filter === "all"} onClick={() => setFilter("all")}>{t("grades.allWork")}</button>
          <button type="button" role="radio" aria-checked={filter === "graded"} onClick={() => setFilter("graded")}>{t("grades.gradedOnly")}</button>
        </div>
      </div>
      {!work.data ? <p className="loading" role="status">{t("common.loading")}</p> : courses.length === 0 ? <p className="empty">{t("grades.noCourses")}</p> : (
        <div className={s.stack}>
          {courses.map(({ c, rows, total, graded }) => {
            const sum = graded.length ? sumPoints(graded.map((g) => g.score ?? "0")) : null;
            const max = graded.length ? sumPoints(graded.map((g) => g.max_points)) : null;
            return (
              <section key={c.id} aria-label={c.title} className="card card-list">
                <div className={s.head}>
                  <span className="tag">{c.code}</span>
                  <Link href={courseHref(c.id)} className={s.course}>{c.title}</Link>
                  <span className="muted" style={{ fontSize: 14 }}>{t("grades.gradedCount", { n: graded.length, total })}</span>
                  {sum && <span style={{ whiteSpace: "nowrap" }}><span style={{ fontSize: 18, fontWeight: 600 }}>{sum}</span><span className="muted" style={{ fontSize: 13 }}> / {max}</span></span>}
                </div>
                <div role="table" aria-label={t("grades.tableLabel", { title: c.title })}>
                  {rows.map((r: MyWorkItem) => {
                    const p = statusPill(r.status);
                    return (
                      <Link key={r.assignment_id} href={itemHref(r.course_id, r.assignment_id)} role="row" className={s.row}>
                        <span role="cell" className={s.cellMain}><ItemTile kind="assignment" small /><span className="ellipsis" style={{ fontWeight: 500 }}>{r.title}</span></span>
                        <span role="cell" className={s.cellDate}>{dueShort(r.due_at ?? null, r.class_on, today)}</span>
                        <span role="cell" className={s.cellStatus}><StatusPill tone={p.tone}>{p.label}</StatusPill></span>
                        <span role="cell" className={s.cellScore}>
                          <span className={s.score} style={{ fontWeight: r.score ? 600 : 400, color: r.score ? undefined : "var(--muted)" }}>{r.score ?? "—"}</span><span className="muted" style={{ fontSize: 13 }}> / {r.max_points}</span>
                        </span>
                      </Link>
                    );
                  })}
                  {rows.length === 0 && <p className="muted" style={{ padding: "16px 18px", fontSize: 14 }}>{t("grades.noneHere")}</p>}
                </div>
              </section>
            );
          })}
        </div>
      )}
      <p className="muted" style={{ margin: "16px 2px 0", fontSize: 13 }}>{t("grades.note")}</p>
    </div>
  );
}
