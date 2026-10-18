"use client";

import Link from "next/link";
import { BlockRenderer } from "@/components/doc/BlockRenderer";
import { Icon } from "@/components/Icon";
import { useCourseInfo, useItemMeta } from "@/lib/api/hooks/courses";
import { useMyGrades } from "@/lib/api/hooks/work";
import { now } from "@/lib/clock";
import { ownerName, typeLabel } from "@/lib/course";
import { dayLong, initials, relative, weekdayDate } from "@/lib/format";
import { wholePoints } from "@/lib/points";
import { bodyOf, type Item } from "@/lib/api/types";
import { DueBar } from "./DueBar";
import { ItemFrame, PreviewBanner } from "./ItemFrame";
import { useT } from "@/lib/i18n";
import s from "./Item.module.css";

/** Work done in class (type `no_submission`): nothing to hand in, the teacher grades it afterwards. */
export function NoSubmissionPage({ item, courseId, staff }: { item: Item; courseId: string; staff: boolean }) {
  const t = useT();
  const a = item.assignment!;
  const info = useCourseInfo(courseId);
  const meta = useItemMeta(courseId);
  const grades = useMyGrades(staff ? null : courseId);
  const grade = grades.data?.items.find((g) => g.assignment_id === item.id);
  const classOn = meta.data?.items[item.id]?.class_on ?? null;
  const past = !!classOn && new Date(classOn) < now();
  const owner = ownerName(info.data) ?? t("item.yourTeacher");

  const sentence = classOn ? (
    <>
      <span className={s.barStrong}>{t(past ? "ns.tookPlace" : "ns.inClassOn", { day: weekdayDate(classOn) })}</span>
      {!past && <span className="muted">, {relative(classOn)}</span>}
    </>
  ) : <span className={s.barStrong}>{t("type.inClass")}</span>;

  return (
    <ItemFrame courseId={courseId} itemId={item.id}>
      {({ crumbs }) => (
        <>
          {staff && <PreviewBanner courseId={courseId} itemId={item.id} />}
          {crumbs}
          <div className={s.kind}>
            <span aria-hidden="true" className={s.kindIcon} data-kind="assignment"><Icon name="assignment" size={15} strokeWidth={2} /></span>
            <span className={s.kindLabel} data-kind="assignment">{t("cm.assignment")}</span>
            <span className="muted" style={{ fontSize: 14 }}>{typeLabel(a.type)}</span>
          </div>
          <h1 className={s.title}>{item.title}</h1>
          <DueBar due={null} open points={a.max_points} sentence={sentence} pill={grade ? { label: t("status.graded"), tone: "ok" } : { label: t("status.nothingToSubmit"), tone: "strong" }}>
            {grade && <span className={s.score}><span className={s.scoreNum}>{grade.score}</span><span className="muted" style={{ fontSize: 14 }}> / {a.max_points}</span></span>}
          </DueBar>
          <div className={s.blocks}>
            <section aria-labelledby="instr-h">
              <h2 id="instr-h" className={s.h2}>{t("item.instructions")}</h2>
              <BlockRenderer doc={bodyOf(item)} size={16} />
            </section>
            {!grade && !staff && (
              <section aria-labelledby="next-h" className="card card-pad">
                <h2 id="next-h" style={{ fontSize: 17, fontWeight: 600 }}>{t("ns.nothingTitle")}</h2>
                <p className="muted" style={{ margin: "6px 0 14px", fontSize: 15 }}>
                  {t("ns.nothingLead", { owner })}
                </p>
                <Link href="/calendar" className="btn btn-outline"><Icon name="calendar" size={16} />{t("ns.seeCalendar")}</Link>
              </section>
            )}
            {grade && (
              <section aria-labelledby="grade-h" className={s.panel}>
                <div className={s.gradeTop}>
                  <div className="row" style={{ flexWrap: "wrap", alignItems: "baseline", justifyContent: "space-between", gap: 8 }}>
                    <h2 id="grade-h" style={{ fontSize: 16, fontWeight: 600 }}>{t("item.grade")}</h2>
                    <span className="muted" style={{ fontSize: 13 }}>{t("item.publishedOn", { day: dayLong(grade.published_at) })}</span>
                  </div>
                  <p style={{ marginTop: 6, whiteSpace: "nowrap" }}><span className={s.gradeNum}>{grade.score}</span><span className="muted" style={{ fontSize: 17 }}> / {a.max_points}</span></p>
                </div>
                {grade.feedback && (
                  <div className={s.comment} style={{ display: "flex", gap: 10 }}>
                    <span aria-hidden="true" className={s.avatar} style={{ flex: "none" }}>{initials(info.data?.owner_name ?? "", info.data?.owner_surname ?? "")}</span>
                    <p style={{ fontSize: 15, lineHeight: 1.6 }}><strong>{owner}: </strong>{grade.feedback}</p>
                  </div>
                )}
              </section>
            )}
          </div>
          <p className="sr-only">{t("content.worth", { points: wholePoints(a.max_points) })}</p>
        </>
      )}
    </ItemFrame>
  );
}
