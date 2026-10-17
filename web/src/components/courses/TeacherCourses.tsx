"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMemo, useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useCourseCards, useCreateCourse, useCreateFromTemplate, useTemplates, useTerms } from "@/lib/api/hooks/courses";
import { dayLong, dueLabel, plural } from "@/lib/format";
import type { Template } from "@/lib/api/gaps";
import s from "@/components/Courses.module.css";

const ROLE: Record<string, string> = { owner: "Owner", teacher: "Teacher", assistant: "Assistant", student: "Student" };

export function TeacherCourses() {
  const router = useRouter();
  const cards = useCourseCards();
  const templates = useTemplates();
  const terms = useTerms();
  const create = useCreateCourse();
  const fromTemplate = useCreateFromTemplate();
  const [tab, setTab] = useState<"teaching" | "archived">("teaching");
  const [dialog, setDialog] = useState<null | { kind: "new" } | { kind: "template"; template: Template }>(null);
  const [title, setTitle] = useState("");
  const [termId, setTermId] = useState("");

  const { teaching, archived } = useMemo(() => {
    const all = cards.data ?? [];
    const old = all.filter((c) => c.course.status === "archived" || c.term_ended);
    return { teaching: all.filter((c) => !old.includes(c)), archived: old };
  }, [cards.data]);
  const list = tab === "teaching" ? teaching : archived;
  const byTerm = [...new Set(list.map((c) => c.term_name ?? "No term"))];

  function open(d: NonNullable<typeof dialog>) {
    setTitle(d.kind === "template" ? d.template.course.title : "");
    setTermId("");
    create.reset();
    fromTemplate.reset();
    setDialog(d);
  }

  async function submit() {
    if (!dialog) return;
    const c = dialog.kind === "new" ? await create.run(title, termId || null) : await fromTemplate.run(dialog.template.course.id, title);
    if (c) {
      setDialog(null);
      router.push(`/courses/${c.id}/settings`);
    }
  }
  const error = (dialog?.kind === "new" ? create.error : fromTemplate.error)?.errors.find((e) => e.field === "title")?.message;

  return (
    <div className="container">
      <div className="page-head" style={{ alignItems: "center" }}>
        <h1 className="page-title" style={{ flex: "1 1 auto" }}>Courses</h1>
        <div role="tablist" aria-label="Which courses" className="segmented">
          <button type="button" role="tab" aria-selected={tab === "teaching"} onClick={() => setTab("teaching")}>Teaching, {teaching.length}</button>
          <button type="button" role="tab" aria-selected={tab === "archived"} onClick={() => setTab("archived")}>Archived, {archived.length}</button>
        </div>
        <button type="button" className="btn btn-primary" onClick={() => open({ kind: "new" })}>
          <Icon name="plus" size={16} strokeWidth={2.2} />
          New course
        </button>
      </div>

      {!cards.data ? <p className="loading" role="status">Loading…</p> : list.length === 0 ? <p className="empty">{tab === "teaching" ? "You don't teach any course this term." : "No archived courses."}</p> : byTerm.map((t) => (
        <section key={t} style={{ marginBottom: 28 }}>
          <h2 className="section-title">{t}</h2>
          <div className={s.grid}>
            {list.filter((c) => (c.term_name ?? "No term") === t).map((c) => (
              <article key={c.course.id} className={`card ${s.card}`}>
                <div className="row" style={{ justifyContent: "space-between", gap: 8 }}>
                  <span className={s.code}>{c.code || "No code"}</span>
                  <span className="muted" style={{ fontSize: 13 }}>{ROLE[c.my_role]}</span>
                </div>
                <Link href={`/courses/${c.course.id}`} className={s.title}>{c.course.title}</Link>
                <p className="muted" style={{ fontSize: 14 }}>
                  {plural(c.student_count, "student")}{c.assistant_count ? `, ${plural(c.assistant_count, "assistant")}` : ""}
                  {c.course.status === "draft" ? ", draft" : ""}
                </p>
                {c.todo.length > 0 && (
                  <ul className={s.todo}>
                    {c.todo.map((td) => (
                      <li key={td.text}>
                        <span aria-hidden="true" className={s.dot} data-kind={td.kind} />
                        <span>{td.due_at ? `${td.text}, ${dueLabel(td.due_at)}` : td.text}</span>
                      </li>
                    ))}
                  </ul>
                )}
                <div className={s.links}>
                  <Link href={`/courses/${c.course.id}`}>Content</Link>
                  <Link href={`/courses/${c.course.id}/grades`}>Gradebook</Link>
                  <Link href={`/courses/${c.course.id}/people`}>People</Link>
                </div>
              </article>
            ))}
          </div>
        </section>
      ))}

      <section aria-labelledby="tpl-h" style={{ marginTop: 8 }}>
        <h2 id="tpl-h" className="section-title">Templates</h2>
        <p className="muted" style={{ margin: "0 2px 10px", fontSize: 14 }}>A new course made from a template gets a full copy of its content. Changing the template later doesn&apos;t change existing courses.</p>
        <div className="card card-list">
          {(templates.data ?? []).map((tp) => (
            <div key={tp.course.id} className={s.tpl}>
              <span className="grow" style={{ flex: "1 1 280px" }}>
                <span style={{ display: "block", fontWeight: 600 }}>{tp.course.title}</span>
                <span className="muted" style={{ display: "block", fontSize: 13 }}>
                  {plural(tp.sections, "section")}, {plural(tp.lectures, "lecture")}, {plural(tp.assignments, "assignment")}. Updated {dayLong(tp.updated_at)}.
                </span>
              </span>
              <Link href={`/courses/${tp.course.id}`} className="btn btn-quiet">Edit template</Link>
              <button type="button" className="btn btn-outline" onClick={() => open({ kind: "template", template: tp })}>Create course from it</button>
            </div>
          ))}
          {templates.data?.length === 0 && <p className="empty">You have no templates yet. Make one from a course&apos;s settings.</p>}
        </div>
      </section>

      {dialog && (
        <Dialog
          title={dialog.kind === "new" ? "New course" : `New course from "${dialog.template.course.title}"`}
          description={dialog.kind === "new" ? "The course starts as a draft. Students see it once you publish it." : "You get a full copy of the template's content, as drafts."}
          onClose={() => setDialog(null)}
          actions={
            <>
              <button type="button" className="btn" onClick={() => setDialog(null)}>Cancel</button>
              <button type="button" className="btn btn-primary" disabled={create.pending || fromTemplate.pending} onClick={submit}>Create course</button>
            </>
          }
        >
          <div className="stack" style={{ gap: 14 }}>
            <div className="field">
              <label htmlFor="course-title">Course title</label>
              <input id="course-title" className="input" value={title} onChange={(e) => setTitle(e.target.value)} aria-invalid={!!error} />
              {error && <p className="hint" style={{ color: "var(--danger)" }}>{error}</p>}
            </div>
            {dialog.kind === "new" && (
              <div className="field">
                <label htmlFor="course-term">Term</label>
                <select id="course-term" className="select" value={termId} onChange={(e) => setTermId(e.target.value)}>
                  <option value="">No term</option>
                  {(terms.data ?? []).map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
                </select>
              </div>
            )}
          </div>
        </Dialog>
      )}
    </div>
  );
}
