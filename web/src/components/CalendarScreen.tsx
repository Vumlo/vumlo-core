"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { Icon } from "@/components/Icon";
import { StatusPill } from "@/components/StatusPill";
import { useMyWork, useTeachingOverview } from "@/lib/api/hooks/work";
import type { WorkStatus } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { statusPill } from "@/lib/course";
import { dayKey, dayKeyLabel, monthName, offsetLabel, parts, relativeCap, timeOf, weekdayShort } from "@/lib/format";
import { useT } from "@/lib/i18n";
import { useMe } from "@/lib/session";
import s from "./Calendar.module.css";
import { itemHref } from "@/lib/routes";

type Ev = { id: string; courseId: string; code: string; courseTitle: string; title: string; at: string; status: WorkStatus | null; href: string };


export function CalendarScreen() {
  const t = useT();
  const me = useMe();
  const student = me.data?.role === "student";
  const work = useMyWork();
  const teaching = useTeachingOverview();
  const today = parts(now());
  const [month, setMonth] = useState({ y: today.y, m: today.m });
  const [view, setView] = useState<"month" | "list">("month");
  const [selected, setSelected] = useState(dayKey(now()));
  const [hidden, setHidden] = useState<Record<string, boolean>>({});

  const { events, courses } = useMemo(() => {
    const evs: Ev[] = [];
    const list: { code: string; title: string }[] = [];
    if (student && work.data) {
      for (const i of work.data.items) {
        const at = i.due_at ?? i.class_on;
        if (!at) continue;
        const c = work.data.courses.find((x) => x.id === i.course_id)!;
        evs.push({ id: i.assignment_id, courseId: c.id, code: c.code, courseTitle: c.title, title: i.title, at, status: i.status, href: itemHref(i.course_id, i.assignment_id) });
        if (!list.some((x) => x.code === c.code)) list.push({ code: c.code, title: c.title });
      }
    } else if (teaching.data) {
      for (const d of teaching.data.deadlines) {
        evs.push({ id: d.assignment_id, courseId: d.course_id, code: d.course_code, courseTitle: d.course_code, title: d.title, at: d.due_at, status: null, href: itemHref(d.course_id, d.assignment_id) });
        if (!list.some((x) => x.code === d.course_code)) list.push({ code: d.course_code, title: d.course_code });
      }
    }
    return { events: evs.sort((a, b) => a.at.localeCompare(b.at)), courses: list };
  }, [student, work.data, teaching.data]);

  const visible = events.filter((e) => !hidden[e.code]);
  const first = new Date(Date.UTC(month.y, month.m - 1, 1)).getUTCDay();
  const lead = (first + 6) % 7;
  const length = new Date(Date.UTC(month.y, month.m, 0)).getUTCDate();
  const prevLen = new Date(Date.UTC(month.y, month.m - 1, 0)).getUTCDate();
  const cells: { key: string; day: number; out: boolean }[] = [];
  for (let k = 0; k < lead; k++) cells.push({ key: "", day: prevLen - lead + k + 1, out: true });
  for (let d = 1; d <= length; d++) cells.push({ key: `${month.y}-${String(month.m).padStart(2, "0")}-${String(d).padStart(2, "0")}`, day: d, out: false });
  while (cells.length % 7) cells.push({ key: "", day: cells.length - lead - length + 1, out: true });
  const byDay = (key: string) => visible.filter((e) => dayKey(e.at) === key);
  const todayKey = dayKey(now());
  const selEvents = byDay(selected);
  const [sy = 0, sm = 1, sd = 1] = selected.split("-").map(Number);
  const selIso = new Date(Date.UTC(sy, sm - 1, sd, 12)).toISOString();
  const upcoming = visible.filter((e) => dayKey(e.at) >= todayKey);
  const agenda = [...new Set(upcoming.map((e) => dayKey(e.at)))].map((k) => ({ key: k, items: upcoming.filter((e) => dayKey(e.at) === k) }));
  const shift = (delta: number) => setMonth(({ y, m }) => { const t = m - 1 + delta; return { y: y + Math.floor(t / 12), m: ((t % 12) + 12) % 12 + 1 }; });

  const dayTitle = dayKeyLabel;

  return (
    <div className="container">
      <div className={s.head}>
        <h1 className={s.month}>{monthName(month.m)} {month.y}</h1>
        <button type="button" className="btn" onClick={() => { setMonth({ y: today.y, m: today.m }); setSelected(todayKey); }}>{t("date.today.cap")}</button>
        <div className={s.arrows}>
          <button type="button" aria-label={t("calendar.previous")} onClick={() => shift(-1)}><Icon name="chevronLeft" size={16} strokeWidth={2} /></button>
          <button type="button" aria-label={t("calendar.next")} onClick={() => shift(1)}><Icon name="chevronRight" size={16} strokeWidth={2} /></button>
        </div>
        <span style={{ flex: "1 1 auto" }} />
        <div role="radiogroup" aria-label={t("calendar.view")} className="segmented">
          <button type="button" role="radio" aria-checked={view === "month"} onClick={() => setView("month")}>{t("calendar.month")}</button>
          <button type="button" role="radio" aria-checked={view === "list"} onClick={() => setView("list")}>{t("calendar.list")}</button>
        </div>
      </div>
      <div role="group" aria-label={t("calendar.showCourses")} className={s.filters}>
        {courses.map((c) => {
          const on = !hidden[c.code];
          return (
            <button key={c.code} type="button" role="checkbox" aria-checked={on} className={s.chip} title={c.title} onClick={() => setHidden((h) => ({ ...h, [c.code]: on }))}>
              <span aria-hidden="true" className={s.box}><Icon name="check" size={12} strokeWidth={3} /></span>
              {c.code}
            </button>
          );
        })}
        <span className="muted" style={{ marginLeft: "auto", fontSize: 13 }}>{t("calendar.zone", { offset: offsetLabel() })}</span>
      </div>

      {(student ? !work.data : !teaching.data) ? <p className="loading" role="status">{t("common.loading")}</p> : (
        <div className={s.body}>
          <div className={s.main}>
            {view === "month" ? (
              <div role="grid" aria-label={`${monthName(month.m)} ${month.y}`} className="card" style={{ overflow: "hidden" }}>
                <div role="row" className={s.grid}>{[0, 1, 2, 3, 4, 5, 6].map((w) => <div key={w} role="columnheader" className={s.wd}>{weekdayShort(w)}</div>)}</div>
                <div className={s.grid}>
                  {cells.map((c, idx) => {
                    const evs = c.out ? [] : byDay(c.key);
                    const past = !c.out && c.key < todayKey;
                    return (
                      <button key={idx} type="button" role="gridcell" className={s.cell} data-out={c.out} aria-selected={!c.out && c.key === selected} disabled={c.out} onClick={() => setSelected(c.key)}>
                        <span className={s.num} data-today={c.key === todayKey} data-past={past}>{c.day}</span>
                        {evs.map((e) => (
                          <span key={e.id} className={s.event} data-done={e.status === "graded"} title={`${e.title}, ${e.code}, ${timeOf(e.at)}`}>
                            {e.status === "graded" && <Icon name="check" size={11} strokeWidth={3} />}
                            <span className="t">{timeOf(e.at)}</span>
                            <span className="tt ellipsis">{e.title}</span>
                          </span>
                        ))}
                      </button>
                    );
                  })}
                </div>
              </div>
            ) : (
              <div>
                {agenda.length === 0 && <p className="empty">{t("calendar.nothingDue")}</p>}
                {agenda.map((g) => (
                  <section key={g.key} className={s.agendaDay}>
                    <h2>{dayTitle(g.key)} <span className="muted" style={{ fontWeight: 400, fontSize: 14 }}>{relativeCap(g.items[0]!.at)}</span></h2>
                    <div className="card card-list">
                      {g.items.map((e) => {
                        const p = e.status ? statusPill(e.status) : null;
                        return (
                          <Link key={e.id} href={e.href} className={s.agendaRow}>
                            <span className={s.time}>{timeOf(e.at)}</span>
                            <span className="grow"><span style={{ display: "block", fontWeight: 500 }}>{e.title}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>{e.courseTitle}</span></span>
                            {p && <StatusPill tone={p.tone}>{p.label}</StatusPill>}
                          </Link>
                        );
                      })}
                    </div>
                  </section>
                ))}
              </div>
            )}
          </div>
          <aside aria-labelledby="day-h" className={`card card-pad ${s.aside}`}>
            <h2 id="day-h" style={{ fontSize: 17, fontWeight: 600 }}>{dayTitle(selected)}</h2>
            <p className="muted" style={{ fontSize: 14, marginBottom: 10 }}>{relativeCap(selIso)}</p>
            {selEvents.length > 0 ? selEvents.map((e) => {
              const p = e.status ? statusPill(e.status) : null;
              return (
                <Link key={e.id} href={e.href} className={s.dayItem}>
                  <span className="row" style={{ gap: 6, fontSize: 13, fontWeight: 600, color: "var(--asg-fg)" }}><Icon name="assignment" size={14} strokeWidth={2} />{t("calendar.dueAt", { time: timeOf(e.at) })}</span>
                  <span style={{ display: "block", fontWeight: 500, marginTop: 2 }}>{e.title}</span>
                  <span className="muted" style={{ display: "block", fontSize: 13, marginBottom: 6 }}>{e.courseTitle}</span>
                  {p && <StatusPill tone={p.tone}>{p.label}</StatusPill>}
                </Link>
              );
            }) : <p style={{ fontSize: 14 }}>{t("calendar.nothingDay")}</p>}
            <p className="muted" style={{ marginTop: 14, fontSize: 13 }}>{t("calendar.note")}</p>
          </aside>
        </div>
      )}
    </div>
  );
}
