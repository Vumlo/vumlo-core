"use client";

import Link from "next/link";
import { useState } from "react";
import { ItemTile } from "@/components/ItemTile";
import { usePublishGrades } from "@/lib/api/hooks/assignments";
import { useTeachingOverview } from "@/lib/api/hooks/work";
import type { TeachingOverview } from "@/lib/api/gaps";
import { dueLabel, initials, plural } from "@/lib/format";
import s from "./GradingScreen.module.css";

function PublishButton({ row }: { row: TeachingOverview["unpublished"][number] }) {
  const publish = usePublishGrades(row.assignment_id);
  return (
    <>
      <button type="button" className="btn btn-primary" disabled={publish.pending} onClick={() => void publish.run("all")}>Publish {row.draft_count}</button>
      {publish.error && <span role="alert" className="hint" style={{ color: "var(--danger)" }}>{publish.error.detail ?? publish.error.title}</span>}
    </>
  );
}

export function GradingScreen() {
  const o = useTeachingOverview();
  const [course, setCourse] = useState("all");
  const courses = [...new Map([...(o.data?.to_grade ?? []), ...(o.data?.unpublished ?? [])].map((x) => [x.course_id, `${x.course_code} ${x.course_title}`])).entries()];
  const queue = (o.data?.to_grade ?? []).filter((q) => course === "all" || q.course_id === course);
  const drafts = (o.data?.unpublished ?? []).filter((d) => course === "all" || d.course_id === course);
  const draftTotal = drafts.reduce((a, d) => a + d.draft_count, 0);

  return (
    <div className="container">
      <div className="page-head" style={{ alignItems: "center" }}>
        <div style={{ flex: "1 1 auto" }}>
          <h1 className="page-title">Grading</h1>
          <p className="page-sub">Everything waiting for you across your courses.</p>
        </div>
        <select aria-label="Course" className="select" style={{ width: "auto", minHeight: 40 }} value={course} onChange={(e) => setCourse(e.target.value)}>
          <option value="all">All courses</option>
          {courses.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
        </select>
      </div>
      {!o.data ? <p className="loading" role="status">Loading…</p> : (
        <>
          <section aria-labelledby="queue-h">
            <div className="row" style={{ justifyContent: "space-between", margin: "0 2px 10px" }}>
              <h2 id="queue-h" style={{ fontSize: 18, fontWeight: 600 }}>Waiting for a grade<span className={s.count}>{queue.length}</span></h2>
              <span className="muted" style={{ fontSize: 13 }}>Oldest first</span>
            </div>
            <div className="card" style={{ overflowX: "auto" }}>
              <div role="table" aria-label="Submissions waiting for a grade">
                <div role="row" className={`${s.row} ${s.head}`}>
                  <span role="columnheader">Student</span><span role="columnheader">Assignment</span><span role="columnheader">Submitted</span><span role="columnheader" />
                </div>
                {queue.map((q) => (
                  <div key={`${q.assignment_id}${q.student_id}`} role="row" className={s.row}>
                    <span role="cell" className={s.who}><span aria-hidden="true" className={s.avatar}>{initials(q.student_name, q.student_surname)}</span><span style={{ fontWeight: 500 }}>{q.student_name} {q.student_surname}</span></span>
                    <span role="cell"><span style={{ display: "block" }}>{q.assignment_title}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>{q.course_code} {q.course_title}</span></span>
                    <span role="cell"><span style={{ display: "block", fontSize: 14 }}>{dueLabel(q.submitted_at)}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>Attempt {q.attempt}{q.max_attempts ? ` of ${q.max_attempts}` : ""}</span></span>
                    <span role="cell"><Link href={`/courses/${q.course_id}/grades`} className="btn btn-outline" style={{ minHeight: 36 }}>Grade</Link></span>
                  </div>
                ))}
                {queue.length === 0 && <p className="empty">Nothing is waiting for a grade.</p>}
              </div>
            </div>
          </section>
          <section aria-labelledby="drafts-h" style={{ marginTop: 32 }}>
            <h2 id="drafts-h" style={{ fontSize: 18, fontWeight: 600, margin: "0 2px 4px" }}>Grades not published yet<span className={s.count}>{draftTotal}</span></h2>
            <p className="muted" style={{ margin: "0 2px 10px", fontSize: 14 }}>Students don&apos;t see these grades until you publish them.</p>
            <div className="card">
              {drafts.map((d) => (
                <div key={d.assignment_id} className={s.drafts}>
                  <ItemTile kind="assignment" small />
                  <span className="grow" style={{ flex: "1 1 240px" }}>
                    <span style={{ display: "block", fontWeight: 500 }}>{d.assignment_title}</span>
                    <span className="muted" style={{ display: "block", fontSize: 13 }}>{d.course_code} {d.course_title}{d.auto_scored ? ", scored automatically" : ""}</span>
                  </span>
                  <span style={{ fontSize: 14 }}>{plural(d.draft_count, "draft grade")}</span>
                  <Link href={`/courses/${d.course_id}/grades`} className="btn btn-quiet">Review in gradebook</Link>
                  <PublishButton row={d} />
                </div>
              ))}
              {drafts.length === 0 && <p className="empty">No draft grades are waiting.</p>}
            </div>
          </section>
        </>
      )}
    </div>
  );
}
