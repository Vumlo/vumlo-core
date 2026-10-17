"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useMemo, useState } from "react";
import { Icon } from "@/components/Icon";
import { ItemTile } from "@/components/ItemTile";
import { useCourseCards } from "@/lib/api/hooks/courses";
import { shortDay } from "@/lib/format";
import s from "@/components/Courses.module.css";

export function StudentCourses() {
  const params = useSearchParams();
  const cards = useCourseCards();
  const [tab, setTab] = useState<"current" | "past">("current");
  const [q, setQ] = useState(params.get("q") ?? "");

  const { current, past } = useMemo(() => {
    const all = (cards.data ?? []).filter((c) => `${c.code} ${c.course.title}`.toLowerCase().includes(q.trim().toLowerCase()));
    const cur = all.filter((c) => !c.term_ended && c.course.status !== "archived");
    const old = all.filter((c) => !cur.includes(c));
    const terms = [...new Set(old.map((c) => c.term_name ?? "Earlier"))];
    return { current: cur, past: terms.map((t) => ({ term: t, items: old.filter((c) => (c.term_name ?? "Earlier") === t) })) };
  }, [cards.data, q]);
  const pastCount = past.reduce((a, g) => a + g.items.length, 0);
  const term = current[0]?.term_name;

  return (
    <div className="container">
      <div className="page-head" style={{ alignItems: "center" }}>
        <h1 className="page-title" style={{ flex: "1 1 auto" }}>Courses</h1>
        <label className="search" style={{ flex: "0 1 240px", background: "var(--surface)" }}>
          <Icon name="search" size={16} />
          <input type="search" aria-label="Filter courses" placeholder="Filter by name or code" value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
        <div role="tablist" aria-label="Which courses" className="segmented">
          <button type="button" role="tab" aria-selected={tab === "current"} onClick={() => setTab("current")}>Current, {current.length}</button>
          <button type="button" role="tab" aria-selected={tab === "past"} onClick={() => setTab("past")}>Past, {pastCount}</button>
        </div>
      </div>

      {!cards.data ? <p className="loading" role="status">Loading…</p> : tab === "current" ? (
        <section aria-labelledby="term-h">
          <h2 id="term-h" className="section-title">{term ?? "This term"}</h2>
          {current.length === 0 ? <p className="empty">No current courses{q ? " match your filter" : ""}.</p> : (
            <div className={s.grid}>
              {current.map((c) => (
                <Link key={c.course.id} href={`/courses/${c.course.id}`} className={`card ${s.card}`}>
                  <span className={s.code}>{c.code}</span>
                  <span className={s.title}>{c.course.title}</span>
                  <span className="muted" style={{ fontSize: 14 }}>{c.owner_name}</span>
                  {c.progress && (
                    <span className={s.progress}>
                      <span className="row" style={{ justifyContent: "space-between", gap: 12, fontSize: 14 }}>
                        <span>{c.progress.submitted} of {c.progress.total} assignments submitted</span>
                        <span className="muted">{c.progress.graded} graded</span>
                      </span>
                      <span role="meter" aria-label="Assignments submitted" aria-valuemin={0} aria-valuemax={c.progress.total} aria-valuenow={c.progress.submitted} className="meter" style={{ marginTop: 8 }}>
                        <span style={{ width: `${c.progress.total ? Math.round((c.progress.submitted / c.progress.total) * 100) : 0}%` }} />
                      </span>
                    </span>
                  )}
                  <span className={s.next}>
                    <ItemTile kind="assignment" small />
                    <span className="ellipsis">{c.next ? `Next: ${c.next.title}, ${shortDay(c.next.due_at)}` : "Nothing due"}</span>
                  </span>
                </Link>
              ))}
            </div>
          )}
        </section>
      ) : (
        <>
          {past.length === 0 && <p className="empty">No past courses.</p>}
          {past.map((g) => (
            <section key={g.term} style={{ marginBottom: 28 }}>
              <h2 className="section-title">{g.term}</h2>
              <div className="card card-list">
                {g.items.map((c) => (
                  <Link key={c.course.id} href={`/courses/${c.course.id}`} className={s.pastRow}>
                    <span className={s.pastCode}>{c.code}</span>
                    <span className="grow">
                      <span style={{ display: "block", fontWeight: 500 }}>{c.course.title}</span>
                      <span className="muted" style={{ display: "block", fontSize: 13 }}>{c.owner_name}</span>
                    </span>
                    <span className="muted" style={{ flex: "none", fontSize: 13 }}>Read only</span>
                    <Icon name="chevronRight" size={16} strokeWidth={2} style={{ color: "var(--muted)", flex: "none" }} />
                  </Link>
                ))}
              </div>
            </section>
          ))}
          <p className="muted" style={{ margin: "0 2px", fontSize: 14 }}>Past courses stay open for reading. You can see your published grades but can&apos;t submit work.</p>
        </>
      )}
    </div>
  );
}
