"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { errorText } from "@/lib/api/client";
import { useAllCourses, type AllCoursesFilter } from "@/lib/api/hooks/admin";
import { useTerms } from "@/lib/api/hooks/courses";
import { now } from "@/lib/clock";
import { useT } from "@/lib/i18n";
import { courseHref } from "@/lib/routes";

/** Every course of the institution for an administrator, who may open and edit any of them. The server narrows and pages the list. */
export function AllCourses() {
  const t = useT();
  const terms = useTerms();
  const [term, setTerm] = useState<string | undefined>();
  const [status, setStatus] = useState<AllCoursesFilter["status"]>("all");
  const [text, setText] = useState("");
  const [q, setQ] = useState("");
  // The server is asked once the typing has paused.
  useEffect(() => {
    const timer = setTimeout(() => setQ(text.trim()), 300);
    return () => clearTimeout(timer);
  }, [text]);

  const today = now().toISOString().slice(0, 10);
  const current = terms.data?.find((tm) => tm.starts_on <= today && today <= tm.ends_on)?.id ?? "";
  const termId = term ?? current;
  const courses = useAllCourses({ termId, status, q });

  if (!terms.data) return <p className="loading" role="status">{t("common.loading")}</p>;
  return (
    <>
      <div className="page-head" style={{ alignItems: "center" }}>
        <h1 className="page-title" style={{ flex: "1 1 auto" }}>{t("nav.courses")}</h1>
      </div>
      <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 12 }}>
        <input type="search" aria-label={t("ac.search")} placeholder={t("ac.search")} className="input" style={{ flex: "1 1 240px", minHeight: 40 }} value={text} onChange={(e) => setText(e.target.value)} />
        <select aria-label={t("newCourse.term")} className="select" style={{ width: "auto", minHeight: 40 }} value={termId} onChange={(e) => setTerm(e.target.value)}>
          <option value="">{t("ac.allTerms")}</option>
          {terms.data.map((tm) => <option key={tm.id} value={tm.id}>{tm.name}</option>)}
        </select>
        <select aria-label={t("ap.status")} className="select" style={{ width: "auto", minHeight: 40 }} value={status} onChange={(e) => setStatus(e.target.value as AllCoursesFilter["status"])}>
          <option value="all">{t("ac.allStatuses")}</option>
          <option value="published">{t("ap.active")}</option>
          <option value="draft">{t("common.draft")}</option>
          <option value="archived">{t("courses.archived")}</option>
        </select>
      </div>
      {courses.error && <p role="alert" className="alert alert-danger">{errorText(courses.error)}</p>}
      {!courses.data ? <p className="loading" role="status">{t("common.loading")}</p> : (
        <div className="card table-wrap">
          <table className="table">
            <thead><tr><th scope="col">{t("search.kind.course")}</th><th scope="col">{t("newCourse.term")}</th><th scope="col">{t("cs.owner")}</th><th scope="col" className="num">{t("people.students")}</th><th scope="col">{t("ap.status")}</th></tr></thead>
            <tbody>
              {courses.data.map((c) => (
                <tr key={c.course_id}>
                  <th scope="row"><span className="tag" style={{ marginRight: 8 }}>{c.code}</span><Link href={courseHref(c.course_id)} style={{ fontWeight: 500 }}>{c.title}</Link></th>
                  <td>{c.term_name ?? "–"}</td>
                  <td>{c.owner_name} {c.owner_surname}</td>
                  <td className="num">{c.student_count}</td>
                  <td>{c.status === "archived" ? <span className="pill pill-dashed">{t("courses.archived")}</span> : c.status === "draft" ? <span className="pill">{t("common.draft")}</span> : <span className="pill pill-ok">{t("ap.active")}</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {courses.data.length === 0 && <p className="empty">{t("ac.none")}</p>}
        </div>
      )}
      {courses.hasMore && (
        <div className="row" style={{ justifyContent: "center", gap: 12, marginTop: 12 }}>
          <button type="button" className="btn" disabled={courses.loadingMore} onClick={() => void courses.loadMore()}>{t("ac.more")}</button>
          {courses.moreFailed && <p role="alert" className="muted">{t("ac.moreFailed")}</p>}
        </div>
      )}
    </>
  );
}
