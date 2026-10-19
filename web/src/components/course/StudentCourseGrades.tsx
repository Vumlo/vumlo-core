"use client";

import Link from "next/link";
import { ItemTile } from "@/components/ItemTile";
import { StatusPill } from "@/components/StatusPill";
import { useContentTree, useCourseInfo, useItemMeta } from "@/lib/api/hooks/courses";
import { useMyGrades } from "@/lib/api/hooks/work";
import { now } from "@/lib/clock";
import { sections, isAssignment, statusPill } from "@/lib/course";
import { dayLabel, dueSentence, initials, plural } from "@/lib/format";
import { sumPoints } from "@/lib/points";
import { CourseHeader } from "./CourseHeader";
import { CourseOutline } from "./CourseOutline";
import { useT } from "@/lib/i18n";
import c from "./Course.module.css";
import s from "@/components/Grades.module.css";
import { itemHref } from "@/lib/routes";

export function StudentCourseGrades({ courseId }: { courseId: string }) {
  const t = useT();
  const tree = useContentTree(courseId);
  const meta = useItemMeta(courseId);
  const grades = useMyGrades(courseId);
  const info = useCourseInfo(courseId);
  const m = meta.data?.items ?? {};
  const g = grades.data?.items ?? [];
  const asg = tree.data ? tree.data.modules.flatMap((x) => [...x.submodules.flatMap((y) => y.items), ...x.items]).filter((i) => isAssignment(i) && i.assignment!.type !== "no_submission" && i.assignment!.max_points !== "0.00") : [];
  const gradedIds = new Set(g.map((x) => x.assignment_id));
  const open = asg.filter((i) => !gradedIds.has(i.id));
  const [n = "", ...r] = (info.data?.owner_name ?? "").replace(/^Dr\. /, "").split(" ");
  const today = now();

  return (
    <div className="container">
      <div className={c.frame}>
        {tree.data && <CourseOutline courseId={courseId} tree={tree.data} meta={m} />}
        <div className={c.main}>
          <CourseHeader courseId={courseId} />
          {!tree.data || !grades.data ? <p className="loading" role="status">{t("common.loading")}</p> : (
            <>
              <div className={`card ${s.summary}`}>
                <div>
                  <p className="muted" style={{ fontSize: 13 }}>{t("cg.fromGraded")}</p>
                  <p style={{ marginTop: 2, whiteSpace: "nowrap" }}>
                    <span style={{ fontSize: 30, lineHeight: 1.1, fontWeight: 600 }}>{g.length ? sumPoints(g.map((x) => x.score ?? "0")) : "—"}</span>
                    <span className="muted" style={{ fontSize: 15 }}> / {g.length ? sumPoints(g.map((x) => x.max_points)) : "—"}</span>
                  </p>
                </div>
                <div>
                  <p className="muted" style={{ fontSize: 13 }}>{t("status.graded")}</p>
                  <p style={{ marginTop: 2, fontSize: 17, fontWeight: 600 }}>{t("cg.gradedOf", { n: g.length, total: plural(g.length + open.length, "assignment") })}</p>
                </div>
                <div>
                  <p className="muted" style={{ fontSize: 13 }}>{t("cg.stillOpen")}</p>
                  <p style={{ marginTop: 2, fontSize: 17, fontWeight: 600 }}>{t("cg.openPoints", { points: sumPoints(open.map((i) => i.assignment!.max_points)) })}</p>
                </div>
                <p className="muted" style={{ flex: "1 1 220px", fontSize: 13 }}>{t("cg.note")}</p>
              </div>
              {sections(tree.data).filter((sec) => sec.items.some(isAssignment)).map((sec) => (
                <section key={sec.id} style={{ marginTop: 28 }}>
                  <h2 className="section-title">{sec.title}</h2>
                  <div className="card card-list">
                    {sec.items.filter(isAssignment).map((i) => {
                      const grade = g.find((x) => x.assignment_id === i.id);
                      const status = m[i.id]?.status ?? "not_started";
                      const p = statusPill(status);
                      const a = i.assignment!;
                      return (
                        <div key={i.id} className={s.sectionRow}>
                          <Link href={itemHref(courseId, i.id)} className={s.link}>
                            <span className={s.cellMain}>
                              <ItemTile kind="assignment" />
                              <span style={{ minWidth: 0 }}>
                                <span style={{ display: "block", fontWeight: 500 }}>{i.title}</span>
                                <span className="muted" style={{ display: "block", fontSize: 13 }}>
                                  {a.due_at ? t(new Date(a.due_at) < today ? "bar.closed" : "bar.due", { when: dueSentence(a.due_at) }) : m[i.id]?.class_on ? t("due.inClass", { day: dayLabel(m[i.id]!.class_on!) }) : t("due.none")}
                                </span>
                              </span>
                            </span>
                            <span className={s.cellStatus}><StatusPill tone={p.tone}>{grade ? t("row.gradedOn", { day: dayLabel(grade.published_at) }) : p.label}</StatusPill></span>
                            <span className={s.cellScore}>
                              <span style={{ fontSize: 17, fontWeight: grade ? 600 : 400, color: grade ? undefined : "var(--muted)" }}>{grade?.score ?? "—"}</span><span className="muted" style={{ fontSize: 13 }}> / {a.max_points}</span>
                            </span>
                          </Link>
                          {grade?.feedback && (
                            <div className={s.comment}>
                              <span aria-hidden="true" className={s.avatar}>{initials(n, r.join(" "))}</span>
                              <span><strong>{info.data?.owner_name}: </strong>{grade.feedback}</span>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </section>
              ))}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
