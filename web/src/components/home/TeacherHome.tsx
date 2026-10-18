"use client";

import Link from "next/link";
import { useCourseCards } from "@/lib/api/hooks/courses";
import { useTeachingOverview } from "@/lib/api/hooks/work";
import { now } from "@/lib/clock";
import { useT } from "@/lib/i18n";
import { todoText } from "@/lib/course";
import { daysFromToday, dueLabel, fullDate, greeting, initials, plural, shortDay } from "@/lib/format";
import type { User } from "@/lib/session";
import { translate } from "@/lib/i18n/core";
import { WidgetGrid, type WidgetDef } from "./WidgetGrid";
import h from "./Home.module.css";

const linkStyle = { flex: "none", display: "flex", alignItems: "center", minHeight: 36, padding: "0 8px", fontSize: 13, color: "var(--accent-fg)" } as const;

function Loading() {
  return <p className={h.empty} role="status">{translate("common.loading")}</p>;
}

export function TeacherHome({ user }: { user: User }) {
  const t = useT();
  const overview = useTeachingOverview();
  const cards = useCourseCards();
  const today = now();

  const defs: WidgetDef[] = (() => {
    const o = overview.data;
    const drafts = (o?.unpublished ?? []).reduce((a, u) => a + u.draft_count, 0);
    const oldest = o?.to_grade[0];
    const firstContent = o?.unpublished_content[0];
    return [
      {
        id: "grade", title: t("home.t.grade"), badge: o?.to_grade.length ?? null, desc: t("home.t.gradeDesc"), sizes: ["2x2"],
        link: <Link href="/grading" style={linkStyle}>{t("home.openGrading")}</Link>,
        render: () => !o ? <Loading /> : o.to_grade.length === 0 ? <p className={h.empty}>{t("grading.nothing")}</p> : (
          <div>
            {o.to_grade.slice(0, 4).map((q) => (
              <Link key={`${q.assignment_id}${q.student_id}`} href={`/courses/${q.course_id}/grades`} className={h.queue}>
                <span aria-hidden="true" className={h.initials}>{initials(q.student_name, q.student_surname)}</span>
                <span className={h.rowMain}>
                  <span className={h.rowTitle}>{q.student_name} {q.student_surname}</span>
                  <span className={h.rowSub}>{q.assignment_title}, {q.course_code}</span>
                </span>
                <span className="muted" style={{ flex: "none", fontSize: 13 }}>{shortDay(q.submitted_at)}</span>
              </Link>
            ))}
            {oldest && <p className="muted" style={{ padding: "8px 16px", fontSize: 13 }}>{t.plural("home.oldestWaited", Math.max(0, -daysFromToday(oldest.submitted_at)))}</p>}
          </div>
        ),
      },
      {
        id: "pub", title: t("home.t.pub"), desc: t("home.t.pubDesc"), sizes: ["1x1"],
        render: () => !o ? <Loading /> : (
          <div className={h.pad}>
            {drafts === 0 ? <p className="muted">{t("grading.noDrafts")}</p> : (
              <>
                <p><span className={h.big}>{plural(drafts, "draft grade")}</span> {t("home.inAssignments", { n: plural(o.unpublished.length, "assignment") })}</p>
                <p className="muted" style={{ fontSize: 13 }}>{t("home.studentsCantSee")}</p>
                <Link href="/grading" style={{ ...linkStyle, marginTop: "auto", padding: 0 }}>{t("home.reviewPublish")}</Link>
              </>
            )}
          </div>
        ),
      },
      {
        id: "next", title: t("home.t.next"), desc: t("home.t.nextDesc"), sizes: ["1x2"],
        render: () => !o ? <Loading /> : o.deadlines.length === 0 ? <p className={h.empty}>{t("home.noDeadlines")}</p> : (
          <div>
            {o.deadlines.slice(0, 4).map((d) => (
              <Link key={d.assignment_id} href={`/courses/${d.course_id}/items/${d.assignment_id}`} className={h.deadline}>
                <span className={h.deadWhen}>{dueLabel(d.due_at)}</span>
                <span className={h.rowTitle} style={{ fontSize: 14 }}>{d.title}</span>
                <span className={h.rowSub} style={{ fontSize: 12 }}>{d.course_code}, {d.not_open ? t("home.notOpen") : t("home.submittedOf", { n: d.submitted, total: d.total })}</span>
              </Link>
            ))}
          </div>
        ),
      },
      {
        id: "content", title: t("home.t.content"), desc: t("home.t.contentDesc"), sizes: ["1x1"],
        render: () => !o ? <Loading /> : (
          <div className={h.pad}>
            {o.unpublished_content.slice(0, 2).map((c) => (
              <p key={`${c.item_id}${c.reason}`} style={{ fontSize: 14 }}>
                <span style={{ fontWeight: 600 }}>{c.title}</span>, {c.reason === "edits" ? t("home.edits") : c.course_code}
              </p>
            ))}
            {o.unpublished_content.length === 0 && <p className="muted">{t("home.allPublished")}</p>}
            {firstContent && <Link href={`/courses/${firstContent.course_id}`} style={{ ...linkStyle, marginTop: "auto", padding: 0 }}>{t("home.openContent")}</Link>}
          </div>
        ),
      },
      {
        id: "courses", title: t("home.myCourses"), desc: t("home.t.coursesDesc"), sizes: ["4x1", "2x2"], phone: false,
        link: <Link href="/courses" style={linkStyle}>{t("home.allCourses")}</Link>,
        render: (w) => !cards.data ? <Loading /> : (
          <div className={h.courses} style={{ gridTemplateColumns: `repeat(${w === 4 ? 2 : 1}, minmax(0, 1fr))` }}>
            {cards.data.filter((c) => !c.term_ended && c.course.status !== "archived").map((c) => (
              <Link key={c.course.id} href={`/courses/${c.course.id}`} className={h.course}>
                <span className="row" style={{ justifyContent: "space-between", gap: 8 }}>
                  <span className={h.code}>{c.code}</span>
                  <span className="muted" style={{ fontSize: 12 }}>{plural(c.student_count, "student")}</span>
                </span>
                <span className={h.rowTitle} style={{ marginTop: 4, fontSize: 14, fontWeight: 600 }}>{c.course.title}</span>
                <span className={h.rowSub} style={{ fontSize: 12 }}>{c.todo.filter((td) => td.kind === "to_grade" || td.kind === "draft_grades").map(todoText).join(", ") || t("home.nothingWaiting")}</span>
              </Link>
            ))}
          </div>
        ),
      },
    ];
  })();

  return (
    <div className="container">
      <WidgetGrid
        storageKey="vumlo.home.teacher"
        defs={defs}
        defaultOrder={["grade", "pub", "next", "content", "courses"]}
        customizeSlot={(button) => (
          <div className="page-head">
            <div>
              <p className="eyebrow">{fullDate(today)}</p>
              <h1 className="page-title">{greeting(today)}, {user.name}</h1>
            </div>
            <span className="desktop-only">{button}</span>
          </div>
        )}
      />
    </div>
  );
}
