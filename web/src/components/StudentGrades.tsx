"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { ItemTile } from "@/components/ItemTile";
import { StatusPill } from "@/components/StatusPill";
import { useMyWork } from "@/lib/api/hooks/work";
import type { MyWorkItem } from "@/lib/api/gaps";
import { now } from "@/lib/clock";
import { dueShort, statusPill } from "@/lib/course";
import { sumPoints } from "@/lib/points";
import s from "./Grades.module.css";

/** Grades across all courses. Only published grades carry a score; everything else shows its state. */
export function StudentGrades() {
  const work = useMyWork();
  const [filter, setFilter] = useState<"all" | "graded">("all");
  const terms = useMemo(() => [...new Set((work.data?.courses ?? []).map((c) => c.term_name ?? "No term"))].sort().reverse(), [work.data]);
  const [term, setTerm] = useState<string>("");
  const activeTerm = term || terms[0] || "";
  const today = now();

  const courses = (work.data?.courses ?? [])
    .filter((c) => (c.term_name ?? "No term") === activeTerm)
    .map((c) => {
      const rows = (work.data?.items ?? []).filter((i) => i.course_id === c.id && i.type !== "form");
      const graded = rows.filter((r) => r.status === "graded");
      return { c, rows: rows.filter((r) => filter === "all" || r.status === "graded"), total: rows.length, graded };
    });

  return (
    <div className="container">
      <div className="page-head" style={{ alignItems: "center" }}>
        <div style={{ flex: "1 1 auto" }}>
          <h1 className="page-title">Grades</h1>
          <p className="page-sub">{activeTerm}. You see a grade once your teacher publishes it.</p>
        </div>
        <select aria-label="Term" className="select" style={{ width: "auto", minHeight: 40 }} value={activeTerm} onChange={(e) => setTerm(e.target.value)}>
          {terms.map((t) => <option key={t}>{t}</option>)}
        </select>
        <div role="radiogroup" aria-label="Show" className="segmented">
          <button type="button" role="radio" aria-checked={filter === "all"} onClick={() => setFilter("all")}>All work</button>
          <button type="button" role="radio" aria-checked={filter === "graded"} onClick={() => setFilter("graded")}>Graded only</button>
        </div>
      </div>
      {!work.data ? <p className="loading" role="status">Loading…</p> : courses.length === 0 ? <p className="empty">You have no courses in this term.</p> : (
        <div className={s.stack}>
          {courses.map(({ c, rows, total, graded }) => {
            const sum = graded.length ? sumPoints(graded.map((g) => g.score ?? "0")) : null;
            const max = graded.length ? sumPoints(graded.map((g) => g.max_points)) : null;
            return (
              <section key={c.id} aria-label={c.title} className="card card-list">
                <div className={s.head}>
                  <span className="tag">{c.code}</span>
                  <Link href={`/courses/${c.id}`} className={s.course}>{c.title}</Link>
                  <span className="muted" style={{ fontSize: 14 }}>{graded.length} of {total} graded</span>
                  {sum && <span style={{ whiteSpace: "nowrap" }}><span style={{ fontSize: 18, fontWeight: 600 }}>{sum}</span><span className="muted" style={{ fontSize: 13 }}> / {max}</span></span>}
                </div>
                <div role="table" aria-label={`${c.title} grades`}>
                  {rows.map((r: MyWorkItem) => {
                    const p = statusPill(r.status);
                    return (
                      <Link key={r.assignment_id} href={`/courses/${r.course_id}/items/${r.item_id}`} role="row" className={s.row}>
                        <span role="cell" className={s.cellMain}><ItemTile kind="assignment" small /><span className="ellipsis" style={{ fontWeight: 500 }}>{r.title}</span></span>
                        <span role="cell" className={s.cellDate}>{dueShort(r.due_at, r.class_on, today)}</span>
                        <span role="cell" className={s.cellStatus}><StatusPill tone={p.tone}>{p.label}</StatusPill></span>
                        <span role="cell" className={s.cellScore}>
                          <span className={s.score} style={{ fontWeight: r.score ? 600 : 400, color: r.score ? undefined : "var(--muted)" }}>{r.score ?? "—"}</span><span className="muted" style={{ fontSize: 13 }}> / {r.max_points}</span>
                        </span>
                      </Link>
                    );
                  })}
                  {rows.length === 0 && <p className="muted" style={{ padding: "16px 18px", fontSize: 14 }}>No published grades in this course yet.</p>}
                </div>
              </section>
            );
          })}
        </div>
      )}
      <p className="muted" style={{ margin: "16px 2px 0", fontSize: 13 }}>The total next to each course adds up only the grades you have received so far. Your final course grade follows your institution&apos;s grading scale.</p>
    </div>
  );
}
