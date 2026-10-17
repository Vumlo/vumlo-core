"use client";

import Link from "next/link";
import { Icon } from "@/components/Icon";
import { ItemTile } from "@/components/ItemTile";
import { StatusPill } from "@/components/StatusPill";
import { useContentTree, useCourseInfo, useItemMeta } from "@/lib/api/hooks/courses";
import { useMyGrades } from "@/lib/api/hooks/work";
import type { ItemMeta } from "@/lib/api/gaps";
import type { ItemSummary } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { allItems, dueShort, isAssignment, sections, statusPill, typeLabel } from "@/lib/course";
import { dayLabel, dueSentence, plural, relative, sinceLabel } from "@/lib/format";
import { wholePoints } from "@/lib/points";
import { CourseHeader } from "./CourseHeader";
import { CourseOutline } from "./CourseOutline";
import s from "./Course.module.css";

/** The late-work sentence of an assignment, shared by the course page and the assignment page. */
export function lateRule(a: { accept_late?: boolean | null; late_until?: string | null; due_at?: string | null }): string {
  if (!a.due_at) return "There is no deadline.";
  if (!a.accept_late) return "Late work isn't accepted.";
  return a.late_until ? `Late work is accepted until ${dueSentence(a.late_until)}.` : "Late work is accepted.";
}

export function attemptRule(max: number | null | undefined): string {
  return max == null ? "You can submit as many times as you need." : max === 1 ? "You can submit once." : `You can submit up to ${max} times.`;
}

function AssignmentRow({ courseId, item, meta }: { courseId: string; item: ItemSummary; meta: ItemMeta | undefined }) {
  const a = item.assignment!;
  const status = meta?.status ?? "not_started";
  const pill = statusPill(status);
  const label = status === "graded" ? `${meta?.score ?? "—"} / ${a.max_points}` : pill.label;
  const today = now();
  const rel = status === "graded" && meta?.published_at ? `Graded ${dayLabel(meta.published_at)}` : meta?.class_on ? relative(meta.class_on) : a.due_at ? relative(a.due_at) : "";
  return (
    <Link href={`/courses/${courseId}/items/${item.id}`} className={s.rowAssignment}>
      <ItemTile kind="assignment" />
      <span className={s.rowMain}>
        <span className={s.rowTitle}>{item.title}</span>
        <span className={s.rowMeta}>{typeLabel(a.type)}, {a.max_points === "0.00" ? "not graded" : `${wholePoints(a.max_points)} points`}</span>
      </span>
      <span className={s.due}>
        <span className={s.dueMain}>{dueShort(a.due_at ?? null, meta?.class_on, today)}</span>
        <span className={s.rowMeta}>{rel}</span>
      </span>
      <span className={s.pillCol}><StatusPill tone={status === "in_class" ? "strong" : pill.tone}>{status === "in_class" ? "In class" : label}</StatusPill></span>
    </Link>
  );
}

export function StudentContent({ courseId }: { courseId: string }) {
  const tree = useContentTree(courseId);
  const meta = useItemMeta(courseId);
  const info = useCourseInfo(courseId);
  const grades = useMyGrades(courseId);

  const m = meta.data?.items ?? {};
  const items = tree.data ? allItems(tree.data) : [];
  const upcoming = items
    .filter((i) => isAssignment(i) && i.assignment?.due_at && new Date(i.assignment.due_at) >= now() && ["not_started", "draft"].includes(m[i.id]?.status ?? ""))
    .sort((a, b) => a.assignment!.due_at!.localeCompare(b.assignment!.due_at!))[0];
  const upMeta = upcoming ? m[upcoming.id] : undefined;
  const latest = (grades.data?.items ?? []).slice().sort((a, b) => b.published_at.localeCompare(a.published_at))[0];
  const open = items.filter((i) => isAssignment(i) && !["graded"].includes(m[i.id]?.status ?? "") && i.assignment?.type !== "no_submission").length;

  return (
    <div className="container">
      <div className={s.frame}>
        {tree.data && <CourseOutline courseId={courseId} tree={tree.data} meta={m} />}
        <div className={s.main}>
          <CourseHeader courseId={courseId} />
          {!tree.data ? <p className="loading" role="status">Loading…</p> : (
            <div className={s.body}>
              <div className={s.list}>
                {tree.data.modules.length === 0 && <p className="empty">Your teacher hasn&apos;t published anything in this course yet.</p>}
                {sections(tree.data).map((sec) => {
                  const groups = [
                    { label: "Lectures", swatch: "var(--lec-fg)", list: sec.items.filter((i) => !isAssignment(i)) },
                    { label: "Assignments", swatch: "var(--asg-fg)", list: sec.items.filter(isAssignment) },
                  ].filter((g) => g.list.length > 0);
                  return (
                    <section key={sec.id} id={`sec-${sec.id}`} className={s.section}>
                      <h2 className="section-title">{sec.title}</h2>
                      <div className="card card-list">
                        {groups.map((g) => (
                          <div key={g.label}>
                            <h3 className="group-head">
                              <span aria-hidden="true" className={s.swatch} style={{ background: g.swatch }} />
                              <span>{g.label}</span>
                              <span style={{ fontWeight: 400 }}>{g.list.length}</span>
                            </h3>
                            <div className="card-list">
                              {g.list.map((i) =>
                                isAssignment(i) ? (
                                  <AssignmentRow key={i.id} courseId={courseId} item={i} meta={m[i.id]} />
                                ) : (
                                  <Link key={i.id} href={`/courses/${courseId}/items/${i.id}`} className={s.rowLecture}>
                                    <ItemTile kind="lecture" />
                                    <span className={s.rowMain}>
                                      <span className={s.rowTitle}>{i.title}</span>
                                      <span className={s.rowMeta}>Lecture{m[i.id]?.read_minutes ? `, ${m[i.id]!.read_minutes} min read` : ""}</span>
                                    </span>
                                    <Icon name="chevronRight" size={16} strokeWidth={2} style={{ color: "var(--muted)", flex: "none" }} />
                                  </Link>
                                ),
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    </section>
                  );
                })}
              </div>

              <aside aria-labelledby="next-h" className={s.aside}>
                <div className="card card-pad">
                  <h2 id="next-h" className="card-title">Up next</h2>
                  {upcoming ? (
                    <>
                      <div className="row" style={{ gap: 12, alignItems: "flex-start" }}>
                        <ItemTile kind="assignment" />
                        <div style={{ minWidth: 0 }}>
                          <p style={{ fontSize: 17, lineHeight: 1.3, fontWeight: 600 }}>{upcoming.title}</p>
                          <p style={{ marginTop: 4, fontSize: 14 }}>Due {dueSentence(upcoming.assignment!.due_at!)}, {relative(upcoming.assignment!.due_at!)}.</p>
                        </div>
                      </div>
                      {upMeta?.draft_saved_at && (
                        <p className="muted" style={{ margin: "14px 0 0", fontSize: 14 }}>
                          Your draft was saved {sinceLabel(upMeta.draft_saved_at)}.
                          {upcoming.assignment!.on_deadline === "auto_submit" ? " If you don't submit it, it's submitted for you at the deadline." : ""}
                        </p>
                      )}
                      <Link href={`/courses/${courseId}/items/${upcoming.id}`} className="btn btn-primary" style={{ marginTop: 16, minHeight: 44, width: "100%", fontSize: 15 }}>
                        {upMeta?.draft_saved_at ? "Continue draft" : "Open assignment"}
                      </Link>
                      <ul className={s.facts}>
                        <li>Worth {wholePoints(upcoming.assignment!.max_points)} points</li>
                        <li>{attemptRule(upcoming.assignment!.max_attempts)}</li>
                        <li>{lateRule(upcoming.assignment!)}</li>
                      </ul>
                    </>
                  ) : (
                    <p className="muted" style={{ fontSize: 14 }}>{items.some(isAssignment) ? "You're all caught up. Nothing is due." : "There are no assignments yet."}</p>
                  )}
                </div>

                <div className="card card-pad">
                  <h2 className="card-title">Grades in this course</h2>
                  {latest ? (
                    <>
                      <div className="row" style={{ justifyContent: "space-between", alignItems: "baseline", gap: 12 }}>
                        <span style={{ fontSize: 14 }}>{latest.title}</span>
                        <span style={{ whiteSpace: "nowrap" }}>
                          <span style={{ fontSize: 17, fontWeight: 600 }}>{latest.score}</span>
                          <span className="muted" style={{ fontSize: 13 }}> / {latest.max_points}</span>
                        </span>
                      </div>
                      {open > 0 && <p className="muted" style={{ margin: "8px 0 0", fontSize: 13 }}>{plural(open, "assignment")} {open === 1 ? "hasn't" : "haven't"} been graded yet.</p>}
                    </>
                  ) : (
                    <p className="muted" style={{ fontSize: 14 }}>No grades have been published yet.</p>
                  )}
                  <Link href={`/courses/${courseId}/grades`} style={{ display: "inline-flex", alignItems: "center", minHeight: 40, marginTop: 4, fontSize: 14, color: "var(--accent-fg)" }}>See my grades</Link>
                </div>
                {info.data?.description && <p className="muted" style={{ fontSize: 13 }}>{info.data.description}</p>}
              </aside>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

