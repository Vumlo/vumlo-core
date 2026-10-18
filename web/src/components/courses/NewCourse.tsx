"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState, type FormEvent } from "react";
import { NotFoundScreen } from "@/components/StateScreen";
import { useCreateCourse, useCreateFromTemplate, useTemplates, useTerms } from "@/lib/api/hooks/courses";
import { api } from "@/lib/api/client";
import { invalidate } from "@/lib/query";
import { useT } from "@/lib/i18n";
import { useMe } from "@/lib/session";
import type { Course } from "@/lib/api/types";
import s from "./NewCourse.module.css";

type Start = "empty" | "template";

/** Create a course: title, code, term and description, from nothing or from one of the teacher's templates. */
export function NewCourse() {
  const t = useT();
  const router = useRouter();
  const me = useMe();
  const terms = useTerms();
  const templates = useTemplates();
  const create = useCreateCourse();
  const fromTemplate = useCreateFromTemplate();
  const [start, setStart] = useState<Start>("empty");
  const [templateId, setTemplateId] = useState("");
  const [title, setTitle] = useState("");
  const [code, setCode] = useState("");
  const [termId, setTermId] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);

  if (!me.data) return null;
  // The server decides who may create a course; students are simply not shown the page.
  if (me.data.role === "student") return <NotFoundScreen />;

  const list = templates.data ?? [];
  const chosen = list.find((x) => x.course.id === templateId) ?? list[0];
  const failure = start === "template" ? fromTemplate.error : create.error;
  const err = (f: string) => failure?.errors.find((x) => x.field === f)?.message;
  const generic = failure && failure.errors.length === 0 ? failure.title : undefined;

  function pickTemplate(id: string) {
    const was = list.find((x) => x.course.id === (templateId || list[0]?.course.id));
    setTemplateId(id);
    const next = list.find((x) => x.course.id === id);
    // Keep the title in step with the template until the person writes their own.
    if (next && (title === "" || title === was?.course.title)) setTitle(next.course.title);
  }

  function pickStart(v: Start) {
    setStart(v);
    if (v === "template" && chosen && title === "") setTitle(chosen.course.title);
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    const name = title.trim();
    const course = start === "template" && chosen ? await fromTemplate.run(chosen.course.id, name) : await create.run(name, termId || null);
    if (!course) {
      setSaving(false);
      return;
    }
    // A template copy takes no term or details in one call, so they follow as small updates. If one fails the
    // course still exists, and the settings page it opens on shows what was kept.
    if (start === "template" && termId) await api.patch<Course>(`/courses/${course.id}`, { term_id: termId }).catch(() => undefined);
    if (code.trim() || description.trim()) await api.patch(`/courses/${course.id}/info`, { code: code.trim(), description: description.trim() }).catch(() => undefined);
    invalidate("/me/");
    invalidate(`/courses/${course.id}`);
    router.push(`/courses/${course.id}/settings`);
  }

  return (
    <div className="container">
      <nav aria-label={t("common.breadcrumb")} className="crumbs">
        <Link href="/courses">{t("nav.courses")}</Link>
        <span aria-hidden="true" style={{ padding: "0 6px" }}>/</span>
        <span aria-current="page">{t("newCourse.title")}</span>
      </nav>
      <h1 className="page-title" style={{ margin: "6px 0 4px" }}>{t("newCourse.title")}</h1>
      <p className="muted" style={{ marginBottom: 24 }}>{t("newCourse.lead")}</p>

      <form className={`card ${s.form}`} onSubmit={submit} noValidate>
        {list.length > 0 && (
          <fieldset className={s.start}>
            <legend className="label">{t("newCourse.start")}</legend>
            <div role="radiogroup" aria-label={t("newCourse.start")} className="segmented">
              <button type="button" role="radio" aria-checked={start === "empty"} onClick={() => pickStart("empty")}>{t("newCourse.empty")}</button>
              <button type="button" role="radio" aria-checked={start === "template"} onClick={() => pickStart("template")}>{t("newCourse.fromTemplate")}</button>
            </div>
            {start === "template" && chosen && (
              <div className="field">
                <label htmlFor="nc-template">{t("newCourse.template")}</label>
                <select id="nc-template" className="select" value={chosen.course.id} onChange={(e) => pickTemplate(e.target.value)}>
                  {list.map((x) => <option key={x.course.id} value={x.course.id}>{x.course.title}</option>)}
                </select>
                <p className="hint">{t("newCourse.templateHint", { sections: chosen.sections, lectures: chosen.lectures, assignments: chosen.assignments })}</p>
              </div>
            )}
          </fieldset>
        )}

        <div className="field">
          <label htmlFor="nc-title">{t("newCourse.name")}</label>
          <input id="nc-title" className="input" value={title} onChange={(e) => setTitle(e.target.value)} aria-invalid={!!err("title")} autoFocus />
          {err("title") && <p className="field-error">{err("title")}</p>}
        </div>
        <div className={s.two}>
          <div className="field">
            <label htmlFor="nc-code">{t("newCourse.code")}</label>
            <input id="nc-code" className="input" value={code} onChange={(e) => setCode(e.target.value)} placeholder="CS 201" aria-describedby="nc-code-hint" />
            <p id="nc-code-hint" className="hint">{t("newCourse.codeHint")}</p>
          </div>
          <div className="field">
            <label htmlFor="nc-term">{t("newCourse.term")}</label>
            <select id="nc-term" className="select" value={termId} onChange={(e) => setTermId(e.target.value)} aria-invalid={!!err("term_id")}>
              <option value="">{t("newCourse.noTerm")}</option>
              {(terms.data ?? []).map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
            </select>
            {err("term_id") && <p className="field-error">{err("term_id")}</p>}
          </div>
        </div>
        <div className="field">
          <label htmlFor="nc-desc">{t("newCourse.description")}</label>
          <textarea id="nc-desc" className="textarea" value={description} onChange={(e) => setDescription(e.target.value)} />
          <p className="hint">{t("newCourse.descriptionHint")}</p>
        </div>

        {generic && <p role="alert" className="field-error">{generic}</p>}
        <p className="hint">{t("newCourse.draftNote")}</p>
        <div className={s.actions}>
          <Link href="/courses" className="btn">{t("common.cancel")}</Link>
          <button type="submit" className="btn btn-primary" disabled={saving || !title.trim()}>{t("newCourse.submit")}</button>
        </div>
      </form>
    </div>
  );
}
