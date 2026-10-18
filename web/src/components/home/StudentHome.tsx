"use client";

import Link from "next/link";
import { ItemTile } from "@/components/ItemTile";
import { StatusPill } from "@/components/StatusPill";
import { useCourseCards } from "@/lib/api/hooks/courses";
import { useMyWork } from "@/lib/api/hooks/work";
import type { MyWorkItem } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { statusPill } from "@/lib/course";
import { dayKey, dayLabel, dueLabel, fullDate, greeting, monthName, monthOf, parts, weekdayShort, shortDay, sinceLabel, timeOf } from "@/lib/format";
import type { User } from "@/lib/session";
import { useT } from "@/lib/i18n";
import { translate } from "@/lib/i18n/core";
import { WidgetGrid, type WidgetDef } from "./WidgetGrid";
import h from "./Home.module.css";

const itemHref = (w: Pick<MyWorkItem, "course_id" | "assignment_id">) => `/courses/${w.course_id}/items/${w.assignment_id}`;

function Loading() {
  return <p className={h.empty} role="status">{translate("common.loading")}</p>;
}

export function StudentHome({ user }: { user: User }) {
  const t = useT();
  const work = useMyWork();
  const cards = useCourseCards();
  const today = now();

  const defs: WidgetDef[] = (() => {
    const data = work.data;
    const courseOf = (id: string) => data?.courses.find((c) => c.id === id);
    const upcoming = (data?.items ?? [])
      .filter((i) => i.due_at && new Date(i.due_at) >= today && ["not_started", "draft"].includes(i.status))
      .sort((a, b) => a.due_at!.localeCompare(b.due_at!));
    const graded = (data?.items ?? []).filter((i) => i.status === "graded" && i.published_at).sort((a, b) => b.published_at!.localeCompare(a.published_at!));
    const draft = (data?.items ?? []).filter((i) => i.status === "draft" && i.draft_saved_at).sort((a, b) => b.draft_saved_at!.localeCompare(a.draft_saved_at!))[0];
    const tp = parts(today);
    const monthDue = (data?.items ?? []).filter((i) => i.due_at && parts(i.due_at).y === tp.y && parts(i.due_at).m === tp.m);
    const current = (cards.data ?? []).filter((c) => !c.term_ended && c.course.status === "published");

    return [
      {
        id: "due", title: t("home.s.due"), desc: t("home.s.dueDesc"), sizes: ["2x2", "2x1"],
        link: <Link href="/calendar" style={linkStyle}>{t("home.openCalendar")}</Link>,
        render: (_w, hh) => !data ? <Loading /> : upcoming.length === 0 ? <p className={h.empty}>{t("home.nothingSoon")}</p> : (
          <div>
            {upcoming.slice(0, hh === 2 ? 4 : 2).map((i) => {
              const pill = statusPill(i.status);
              return (
                <Link key={i.assignment_id} href={itemHref(i)} className={h.row}>
                  <ItemTile kind="assignment" small />
                  <span className={h.rowMain}>
                    <span className={h.rowTitle}>{i.title}</span>
                    <span className={h.rowSub}>{courseOf(i.course_id)?.title}</span>
                  </span>
                  <span className={h.rowSide}>
                    <span>{dueLabel(i.due_at!)}</span>
                    <StatusPill tone={pill.tone}>{pill.label}</StatusPill>
                  </span>
                </Link>
              );
            })}
          </div>
        ),
      },
      {
        id: "continue", title: t("home.s.continue"), desc: t("home.s.continueDesc"), sizes: ["1x1"],
        render: () => !data ? <Loading /> : !draft ? <p className={h.empty}>{t("home.noDrafts")}</p> : (
          <div className={h.pad}>
            <div className="row" style={{ gap: 10, alignItems: "flex-start" }}>
              <ItemTile kind="assignment" small />
              <div style={{ minWidth: 0 }}>
                <p className={h.rowTitle} style={{ fontWeight: 600 }}>{draft.title}</p>
                <p className="muted" style={{ fontSize: 13 }}>{t("home.draftSaved", { when: sinceLabel(draft.draft_saved_at!) })}</p>
              </div>
            </div>
            <Link href={itemHref(draft)} className="btn btn-primary" style={{ marginTop: "auto" }}>{t("content.continueDraft")}</Link>
          </div>
        ),
      },
      {
        id: "grades", title: t("home.s.grades"), desc: t("home.s.gradesDesc"), sizes: ["1x2", "2x1"],
        link: <Link href="/grades" style={linkStyle}>{t("home.allGrades")}</Link>,
        render: (_w, hh) => !data ? <Loading /> : graded.length === 0 ? <p className={h.empty}>{t("content.noGrades")}</p> : (
          <div>
            {graded.slice(0, hh === 2 ? 3 : 2).map((g) => (
              <Link key={g.assignment_id} href={itemHref(g)} className={h.grade}>
                <span style={{ minWidth: 0 }}>
                  <span className={h.rowTitle} style={{ fontSize: 14 }}>{g.title}</span>
                  <span className={h.rowSub} style={{ fontSize: 12 }}>{courseOf(g.course_id)?.title}, {dayLabel(g.published_at!)}</span>
                </span>
                <span className={h.score}>
                  <span className={h.scoreNum}>{g.score}</span>
                  <span className={h.scoreMax}> / {g.max_points}</span>
                </span>
              </Link>
            ))}
          </div>
        ),
      },
      {
        id: "month", title: monthName(tp.m), desc: t("home.s.monthDesc"), sizes: ["1x1"], note: t.plural("home.s.deadlines", monthDue.length), phone: false,
        render: () => {
          const first = new Date(Date.UTC(tp.y, tp.m - 1, 1)).getUTCDay();
          const lead = (first + 6) % 7;
          const length = new Date(Date.UTC(tp.y, tp.m, 0)).getUTCDate();
          const due = new Map(monthDue.map((i) => [parts(i.due_at!).d, i.title]));
          const cells = [...Array.from({ length: lead }, () => 0), ...Array.from({ length }, (_, k) => k + 1)];
          while (cells.length % 7) cells.push(0);
          return (
            <div className={h.month}>
              <div aria-hidden="true" className={h.weekdays}>{[0, 1, 2, 3, 4, 5, 6].map((k) => <span key={k}>{weekdayShort(k).charAt(0)}</span>)}</div>
              <div className={h.cells}>
                {cells.map((d, k) => (
                  <span key={k} className={h.cell} aria-label={d ? `${d} ${monthOf(tp.m)}${d === tp.d ? `, ${t("date.today")}` : ""}${due.has(d) ? `, ${t("home.dueTitle", { title: due.get(d)! })}` : ""}` : undefined}>
                    {d ? <span className={h.day} data-kind={d === tp.d ? "today" : due.has(d) ? "due" : undefined} data-past={d < tp.d}>{d}</span> : null}
                  </span>
                ))}
              </div>
            </div>
          );
        },
      },
      {
        id: "courses", title: t("home.myCourses"), desc: t("home.s.coursesDesc"), sizes: ["4x1", "2x2"], phone: false,
        link: <Link href="/courses" style={linkStyle}>{t("home.allCourses")}</Link>,
        render: (w) => !cards.data ? <Loading /> : (
          <div className={h.courses} style={{ gridTemplateColumns: `repeat(${w === 4 ? 4 : 2}, minmax(0, 1fr))` }}>
            {current.slice(0, 4).map((c) => (
              <Link key={c.course.id} href={`/courses/${c.course.id}`} className={h.course}>
                <span className={h.code}>{c.code}</span>
                <span className={h.rowTitle} style={{ marginTop: 4, fontSize: 14, fontWeight: 600 }}>{c.course.title}</span>
                <span className={h.rowSub} style={{ fontSize: 12 }}>{c.owner_name}</span>
                <span className={h.rowTitle} style={{ marginTop: "auto", fontSize: 12, fontWeight: 400 }}>
                  {c.next ? t("courses.next", { title: c.next.title, when: shortDay(c.next.due_at) }) : t("home.nothingWeek")}
                </span>
              </Link>
            ))}
          </div>
        ),
      },
      {
        id: "week", title: t("home.s.week"), desc: t("home.s.weekDesc"), sizes: ["4x1"], phone: false,
        render: () => {
          const days = Array.from({ length: 7 }, (_, k) => new Date(today.getTime() + k * 86_400_000));
          return (
            <div className={h.week}>
              {days.map((d, k) => {
                const key = dayKey(d);
                const list = (data?.items ?? []).filter((i) => i.due_at && dayKey(i.due_at) === key);
                return (
                  <div key={key} className={h.weekDay} data-today={k === 0}>
                    <span style={{ fontSize: 12, fontWeight: k === 0 ? 600 : 500, color: k === 0 ? "var(--accent-fg)" : "var(--muted)" }}>{k === 0 ? t("date.today.cap") : `${weekdayShort(parts(d).wd)} ${parts(d).d}`}</span>
                    {list.map((i) => (
                      <Link key={i.assignment_id} href={itemHref(i)} className={h.weekEvent}>
                        <span className={h.rowTitle} style={{ fontSize: 12 }}>{i.title}</span>
                        <span style={{ display: "block", color: "var(--asg-fg)" }}>{timeOf(i.due_at!)}</span>
                      </Link>
                    ))}
                    {list.length === 0 && <span className="muted" style={{ fontSize: 12 }}>{t("courses.nothingDue")}</span>}
                  </div>
                );
              })}
            </div>
          );
        },
      },
    ];
  })();

  return (
    <div className="container">
      <WidgetGrid
        storageKey="vumlo.home.student"
        defs={defs}
        defaultOrder={["due", "continue", "grades", "month", "courses"]}
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

const linkStyle = { flex: "none", display: "flex", alignItems: "center", minHeight: 36, padding: "0 8px", fontSize: 13, color: "var(--accent-fg)" } as const;

