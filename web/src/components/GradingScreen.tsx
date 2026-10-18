"use client";

import Link from "next/link";
import { useState } from "react";
import { ItemTile } from "@/components/ItemTile";
import { usePublishGrades } from "@/lib/api/hooks/assignments";
import { useTeachingOverview } from "@/lib/api/hooks/work";
import type { TeachingOverview } from "@/lib/api/types";
import { dueLabel, initials, plural } from "@/lib/format";
import { useT } from "@/lib/i18n";
import s from "./GradingScreen.module.css";

function PublishButton({ row }: { row: TeachingOverview["unpublished"][number] }) {
  const t = useT();
  const publish = usePublishGrades(row.assignment_id);
  return (
    <>
      <button type="button" className="btn btn-primary" disabled={publish.pending} onClick={() => void publish.run("all")}>{t("grading.publishN", { n: row.draft_count })}</button>
      {publish.error && <span role="alert" className="hint" style={{ color: "var(--danger)" }}>{publish.error.detail ?? publish.error.title}</span>}
    </>
  );
}

export function GradingScreen() {
  const t = useT();
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
          <h1 className="page-title">{t("nav.grading")}</h1>
          <p className="page-sub">{t("grading.lead")}</p>
        </div>
        <select aria-label={t("search.kind.course")} className="select" style={{ width: "auto", minHeight: 40 }} value={course} onChange={(e) => setCourse(e.target.value)}>
          <option value="all">{t("grading.allCourses")}</option>
          {courses.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
        </select>
      </div>
      {!o.data ? <p className="loading" role="status">{t("common.loading")}</p> : (
        <>
          <section aria-labelledby="queue-h">
            <div className="row" style={{ justifyContent: "space-between", margin: "0 2px 10px" }}>
              <h2 id="queue-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("grading.waiting")}<span className={s.count}>{queue.length}</span></h2>
              <span className="muted" style={{ fontSize: 13 }}>{t("grading.oldest")}</span>
            </div>
            <div className="card" style={{ overflowX: "auto" }}>
              <div role="table" aria-label={t("grading.tableLabel")}>
                <div role="row" className={`${s.row} ${s.head}`}>
                  <span role="columnheader">{t("grading.student")}</span><span role="columnheader">{t("grading.assignment")}</span><span role="columnheader">{t("grading.submitted")}</span><span role="columnheader" />
                </div>
                {queue.map((q) => (
                  <div key={`${q.assignment_id}${q.student_id}`} role="row" className={s.row}>
                    <span role="cell" className={s.who}><span aria-hidden="true" className={s.avatar}>{initials(q.student_name, q.student_surname)}</span><span style={{ fontWeight: 500 }}>{q.student_name} {q.student_surname}</span></span>
                    <span role="cell"><span style={{ display: "block" }}>{q.assignment_title}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>{q.course_code} {q.course_title}</span></span>
                    <span role="cell"><span style={{ display: "block", fontSize: 14 }}>{dueLabel(q.submitted_at)}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>{q.max_attempts ? t("grading.attemptOf", { n: q.attempt, max: q.max_attempts }) : t("grading.attempt", { n: q.attempt })}</span></span>
                    <span role="cell"><Link href={`/courses/${q.course_id}/grades`} className="btn btn-outline" style={{ minHeight: 36 }}>{t("grading.grade")}</Link></span>
                  </div>
                ))}
                {queue.length === 0 && <p className="empty">{t("grading.nothing")}</p>}
              </div>
            </div>
          </section>
          <section aria-labelledby="drafts-h" style={{ marginTop: 32 }}>
            <h2 id="drafts-h" style={{ fontSize: 18, fontWeight: 600, margin: "0 2px 4px" }}>{t("grading.notPublished")}<span className={s.count}>{draftTotal}</span></h2>
            <p className="muted" style={{ margin: "0 2px 10px", fontSize: 14 }}>{t("grading.notPublishedLead")}</p>
            <div className="card">
              {drafts.map((d) => (
                <div key={d.assignment_id} className={s.drafts}>
                  <ItemTile kind="assignment" small />
                  <span className="grow" style={{ flex: "1 1 240px" }}>
                    <span style={{ display: "block", fontWeight: 500 }}>{d.assignment_title}</span>
                    <span className="muted" style={{ display: "block", fontSize: 13 }}>{d.course_code} {d.course_title}{d.auto_scored ? `, ${t("grading.auto")}` : ""}</span>
                  </span>
                  <span style={{ fontSize: 14 }}>{plural(d.draft_count, "draft grade")}</span>
                  <Link href={`/courses/${d.course_id}/grades`} className="btn btn-quiet">{t("grading.review")}</Link>
                  <PublishButton row={d} />
                </div>
              ))}
              {drafts.length === 0 && <p className="empty">{t("grading.noDrafts")}</p>}
            </div>
          </section>
        </>
      )}
    </div>
  );
}
