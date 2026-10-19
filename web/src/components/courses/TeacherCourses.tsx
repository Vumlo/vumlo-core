"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMemo, useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useCourseCards, useCreateFromTemplate, useTemplates } from "@/lib/api/hooks/courses";
import { todoText } from "@/lib/course";
import { dayLong, dueLabel, plural } from "@/lib/format";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import type { TemplateSummary } from "@/lib/api/types";
import s from "@/components/Courses.module.css";
import { courseHref } from "@/lib/routes";

const ROLE: Record<string, MessageKey> = { owner: "role.owner", teacher: "role.teacher", assistant: "role.assistant", student: "role.student" };

export function TeacherCourses() {
  const t = useT();
  const router = useRouter();
  const cards = useCourseCards();
  const templates = useTemplates();
  const fromTemplate = useCreateFromTemplate();
  const [tab, setTab] = useState<"teaching" | "archived">("teaching");
  const [dialog, setDialog] = useState<TemplateSummary | null>(null);
  const [title, setTitle] = useState("");

  const { teaching, archived } = useMemo(() => {
    const all = cards.data ?? [];
    const old = all.filter((c) => c.course.status === "archived" || c.term_ended);
    return { teaching: all.filter((c) => !old.includes(c)), archived: old };
  }, [cards.data]);
  const list = tab === "teaching" ? teaching : archived;
  const byTerm = [...new Set(list.map((c) => c.term_name ?? t("courses.noTerm")))];

  function open(template: TemplateSummary) {
    setTitle(template.course.title);
    fromTemplate.reset();
    setDialog(template);
  }

  async function submit() {
    if (!dialog) return;
    const c = await fromTemplate.run(dialog.course.id, title);
    if (c) {
      setDialog(null);
      router.push(courseHref(c.id, "settings"));
    }
  }
  const error = fromTemplate.error?.errors.find((e) => e.field === "title")?.message;

  return (
    <div className="container">
      <div className="page-head" style={{ alignItems: "center" }}>
        <h1 className="page-title" style={{ flex: "1 1 auto" }}>{t("nav.courses")}</h1>
        <div role="tablist" aria-label={t("courses.which")} className="segmented">
          <button type="button" role="tab" aria-selected={tab === "teaching"} onClick={() => setTab("teaching")}>{t("courses.teaching")}, {teaching.length}</button>
          <button type="button" role="tab" aria-selected={tab === "archived"} onClick={() => setTab("archived")}>{t("courses.archived")}, {archived.length}</button>
        </div>
        <Link href="/courses/new" className="btn btn-primary">
          <Icon name="plus" size={16} strokeWidth={2.2} />
          {t("newCourse.title")}
        </Link>
      </div>

      {!cards.data ? <p className="loading" role="status">{t("common.loading")}</p> : list.length === 0 ? <p className="empty">{tab === "teaching" ? t("courses.noneTeaching") : t("courses.noneArchived")}</p> : byTerm.map((term) => (
        <section key={term} style={{ marginBottom: 28 }}>
          <h2 className="section-title">{term}</h2>
          <div className={s.grid}>
            {list.filter((c) => (c.term_name ?? t("courses.noTerm")) === term).map((c) => (
              <article key={c.course.id} className={`card ${s.card}`}>
                <div className="row" style={{ justifyContent: "space-between", gap: 8 }}>
                  <span className={s.code}>{c.code || t("courses.noCode")}</span>
                  <span className="muted" style={{ fontSize: 13 }}>{t(ROLE[c.my_role] ?? "role.teacher")}</span>
                </div>
                <Link href={courseHref(c.course.id)} className={s.title}>{c.course.title}</Link>
                <p className="muted" style={{ fontSize: 14 }}>
                  {plural(c.student_count, "student")}{c.assistant_count ? `, ${plural(c.assistant_count, "assistant")}` : ""}
                  {c.course.status === "draft" ? `, ${t("common.draft").toLowerCase()}` : ""}
                </p>
                {c.todo.length > 0 && (
                  <ul className={s.todo}>
                    {c.todo.map((td) => (
                      <li key={td.kind}>
                        <span aria-hidden="true" className={s.dot} data-kind={td.kind} />
                        <span>{td.kind === "next_deadline" ? `${todoText(td)}, ${dueLabel(td.due_at)}` : todoText(td)}</span>
                      </li>
                    ))}
                  </ul>
                )}
                <div className={s.links}>
                  <Link href={courseHref(c.course.id)}>{t("course.content")}</Link>
                  <Link href={courseHref(c.course.id, "grades")}>{t("course.gradebook")}</Link>
                  <Link href={courseHref(c.course.id, "people")}>{t("course.people")}</Link>
                </div>
              </article>
            ))}
          </div>
        </section>
      ))}

      <section aria-labelledby="tpl-h" style={{ marginTop: 8 }}>
        <h2 id="tpl-h" className="section-title">{t("courses.templates")}</h2>
        <p className="muted" style={{ margin: "0 2px 10px", fontSize: 14 }}>{t("courses.templatesLead")}</p>
        <div className="card card-list">
          {(templates.data ?? []).map((tp) => (
            <div key={tp.course.id} className={s.tpl}>
              <span className="grow" style={{ flex: "1 1 280px" }}>
                <span style={{ display: "block", fontWeight: 600 }}>{tp.course.title}</span>
                <span className="muted" style={{ display: "block", fontSize: 13 }}>
                  {plural(tp.sections, "section")}, {plural(tp.lectures, "lecture")}, {plural(tp.assignments, "assignment")}. {t("courses.updated", { day: dayLong(tp.updated_at) })}
                </span>
              </span>
              <Link href={courseHref(tp.course.id)} className="btn btn-quiet">{t("courses.editTemplate")}</Link>
              <button type="button" className="btn btn-outline" onClick={() => open(tp)}>{t("courses.fromIt")}</button>
            </div>
          ))}
          {templates.data?.length === 0 && <p className="empty">{t("courses.noTemplates")}</p>}
        </div>
      </section>

      {dialog && (
        <Dialog
          title={t("courses.fromTemplateTitle", { title: dialog.course.title })}
          description={t("courses.fromTemplateLead")}
          onClose={() => setDialog(null)}
          actions={
            <>
              <button type="button" className="btn" onClick={() => setDialog(null)}>{t("common.cancel")}</button>
              <button type="button" className="btn btn-primary" disabled={fromTemplate.pending} onClick={submit}>{t("newCourse.submit")}</button>
            </>
          }
        >
          <div className="stack" style={{ gap: 14 }}>
            <div className="field">
              <label htmlFor="course-title">{t("newCourse.name")}</label>
              <input id="course-title" className="input" value={title} onChange={(e) => setTitle(e.target.value)} aria-invalid={!!error} />
              {error && <p className="hint" style={{ color: "var(--danger)" }}>{error}</p>}
            </div>
          </div>
        </Dialog>
      )}
    </div>
  );
}
