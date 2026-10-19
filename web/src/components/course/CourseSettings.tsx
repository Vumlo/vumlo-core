"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useInstitution } from "@/lib/api/hooks/institution";
import { useArchiveCourse, useContentTree, useCourse, useCourseInfo, useCoursePeople, useDeleteCourse, useMakeTemplate, useTerms, useTransferOwner, useUnlinkGroup, useUpdateCourse, useUpdateCourseInfo } from "@/lib/api/hooks/courses";
import { allItems, isAssignment, linkedGroupName, ownerName } from "@/lib/course";
import { dayLabel, dayLong, initials, plural } from "@/lib/format";
import { CourseHeader } from "./CourseHeader";
import { LinkGroup } from "./People";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import s from "./Settings.module.css";
import { courseHref } from "@/lib/routes";

const NAV = [["general", "cs.general"], ["grading", "cs.grading"], ["groups", "cs.groups"], ["team", "cs.owner"], ["template", "cs.reuse"], ["archive", "cs.archive"]] as const satisfies readonly (readonly [string, MessageKey])[];

export function CourseSettings({ courseId }: { courseId: string }) {
  const t = useT();
  const router = useRouter();
  const course = useCourse(courseId);
  const info = useCourseInfo(courseId);
  const terms = useTerms();
  const institution = useInstitution();
  const people = useCoursePeople(courseId);
  const tree = useContentTree(courseId);
  const updateCourse = useUpdateCourse(courseId);
  const updateInfo = useUpdateCourseInfo(courseId);
  const unlink = useUnlinkGroup(courseId);
  const transfer = useTransferOwner(courseId);
  const makeTemplate = useMakeTemplate(courseId);
  const archive = useArchiveCourse(courseId);
  const del = useDeleteCourse(courseId);
  const [active, setActive] = useState("general");
  const [title, setTitle] = useState<string | null>(null);
  const [code, setCode] = useState<string | null>(null);
  const [termId, setTermId] = useState<string | null>(null);
  const [desc, setDesc] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [dialog, setDialog] = useState<null | "group" | "owner" | "archive" | "delete" | "template">(null);
  const [newOwner, setNewOwner] = useState("");

  useEffect(() => {
    const els = NAV.map(([id]) => document.getElementById(id)).filter((e): e is HTMLElement => !!e);
    const io = new IntersectionObserver((entries) => {
      const top = entries.filter((e) => e.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0];
      if (top) setActive(top.target.id);
    }, { rootMargin: "-90px 0px -60% 0px" });
    els.forEach((e) => io.observe(e));
    return () => io.disconnect();
  }, [info.data]);

  const c = course.data;
  const i = info.data;
  if (!c || !i) return <div className="container"><p className="loading" role="status">{t("common.loading")}</p></div>;

  const tTitle = title ?? c.title;
  const tCode = code ?? i.code;
  const tTerm = termId ?? c.term_id ?? "";
  const tDesc = desc ?? i.description;
  const dirty = title !== null || code !== null || termId !== null || desc !== null;
  const fieldError = (f: string) => (updateCourse.error ?? updateInfo.error)?.errors.find((x) => x.field === f)?.message;
  const items = tree.data ? allItems(tree.data) : [];
  const groups = people.data?.groups ?? [];
  const teachers = (people.data?.people ?? []).filter((p) => p.role === "teacher" && !p.is_owner && !p.former);
  const owner = (people.data?.people ?? []).find((p) => p.is_owner);
  const archived = c.status === "archived";

  async function save() {
    setSaved(false);
    const a = title !== null || termId !== null ? await updateCourse.run({ ...(title !== null ? { title } : {}), ...(termId !== null ? { term_id: termId || null } : {}) }) : true;
    const b = code !== null || desc !== null ? await updateInfo.run({ ...(code !== null ? { code } : {}), ...(desc !== null ? { description: desc } : {}) }) : true;
    if (a && b) { setTitle(null); setCode(null); setTermId(null); setDesc(null); setSaved(true); }
  }
  const close = () => setDialog(null);

  return (
    <div className="container">
      <CourseHeader courseId={courseId} />
      <div className={s.layout}>
        <nav aria-label={t("course.settings")} className={s.nav}>
          {NAV.map(([id, label]) => <a key={id} href={`#${id}`} aria-current={active === id} onClick={() => setActive(id)}>{t(label)}</a>)}
        </nav>
        <div className={s.main}>
          <section id="general" aria-labelledby="general-h" className={`card ${s.section}`}>
            <h2 id="general-h">{t("cs.general")}</h2>
            <p className={s.lead}>{t("cs.generalLead")}</p>
            <div className={s.fields}>
              <div className="field">
                <label htmlFor="f-title">{t("newCourse.name")}</label>
                <input id="f-title" className="input" value={tTitle} onChange={(e) => setTitle(e.target.value)} aria-invalid={!!fieldError("title")} />
                {fieldError("title") && <p className="hint" style={{ color: "var(--danger)" }}>{fieldError("title")}</p>}
              </div>
              <div className={s.two}>
                <div className="field"><label htmlFor="f-code">{t("cs.code")}</label><input id="f-code" className="input" value={tCode} onChange={(e) => setCode(e.target.value)} /></div>
                <div className="field">
                  <label htmlFor="f-term">{t("newCourse.term")}</label>
                  <select id="f-term" className="select" value={tTerm} onChange={(e) => setTermId(e.target.value)}>
                    <option value="">{t("courses.noTerm")}</option>
                    {(terms.data ?? []).map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
                  </select>
                </div>
              </div>
              <div className="field">
                <label htmlFor="f-desc">{t("cs.shortDesc")}</label>
                <textarea id="f-desc" className="textarea" rows={3} value={tDesc} onChange={(e) => setDesc(e.target.value)} aria-invalid={!!fieldError("description")} />
                <p className="hint" style={{ color: tDesc.length > 300 ? "var(--danger)" : undefined }}>{t("cs.chars", { n: tDesc.length })}</p>
                {fieldError("description") && <p className="hint" style={{ color: "var(--danger)" }}>{fieldError("description")}</p>}
              </div>
            </div>
            <div className="row" style={{ gap: 14, marginTop: 18, flexWrap: "wrap" }}>
              <button type="button" className="btn btn-primary" disabled={!dirty || updateCourse.pending || updateInfo.pending} onClick={save}>{t("cs.save")}</button>
              <span role="status" className="muted" style={{ fontSize: 14 }}>{saved ? t("cs.saved") : t("cs.lastSaved", { day: dayLabel(i.updated_at) })}</span>
            </div>
          </section>

          <section id="grading" aria-labelledby="grading-h" className={`card ${s.section}`}>
            <h2 id="grading-h">{t("cs.grading")}</h2>
            <p className={s.lead}>{t("cs.gradingLead")}</p>
            <dl className={s.dl}>
              <dt>{t("cs.scale")}</dt><dd>{institution.data ? t(`ai.scale.${institution.data.grading_scale}`) : "…"}</dd>
              <dt>{t("cs.total")}</dt><dd>{t("cs.totalValue", { points: i.total_points, items: plural(i.graded_items, "graded item") })}</dd>
              <dt>{t("cs.seen")}</dt><dd>{t("cs.seenValue")}</dd>
            </dl>
            <p className="muted" style={{ marginTop: 16, fontSize: 14 }}>{t("cs.scaleNote")}</p>
          </section>

          <section id="groups" aria-labelledby="groups-h" className={`card ${s.section}`}>
            <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-start", gap: 16, flexWrap: "wrap" }}>
              <div><h2 id="groups-h">{t("cs.groups")}</h2><p className={s.lead}>{t("cs.groupsLead")}</p></div>
              <button type="button" className="btn btn-outline" onClick={() => setDialog("group")}><Icon name="plus" size={15} strokeWidth={2.2} />{t("people.linkGroup")}</button>
            </div>
            <ul className={s.list}>
              {groups.map((g) => (
                <li key={g.link_id} className={s.li}>
                  <span className="grow"><span style={{ display: "block", fontWeight: 500 }}>{linkedGroupName(g)}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>{plural(g.student_count, "student")}, {t("people.linkedOn", { day: dayLong(g.linked_at) })}</span></span>
                  <button type="button" className="btn btn-quiet" style={{ color: "var(--danger)" }} onClick={() => void unlink.run(g.link_id)}>{t("people.unlink")}</button>
                </li>
              ))}
              {groups.length === 0 && <li className="muted" style={{ fontSize: 14 }}>{t("people.noGroup")}</li>}
            </ul>
          </section>

          <section id="team" aria-labelledby="team-h" className={`card ${s.section}`}>
            <h2 id="team-h">{t("cs.owner")}</h2>
            <p className={s.lead}>{t("cs.ownerLead")} <Link href={courseHref(courseId, "people")} style={{ color: "var(--accent-fg)" }}>{t("cs.peopleTab")}</Link>.</p>
            {owner && (
              <div className={s.owner}>
                <span aria-hidden="true" className={s.avatar}>{initials(owner.name, owner.surname)}</span>
                <span className="grow"><span style={{ display: "block", fontWeight: 500 }}>{ownerName(i)}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>{t("cs.ownerSince", { day: dayLong(c.created_at) })}</span></span>
                <button type="button" className="btn btn-outline" onClick={() => { setNewOwner(""); transfer.reset(); setDialog("owner"); }}>{t("cs.transfer")}</button>
              </div>
            )}
            <p className="muted" style={{ fontSize: 14 }}>{t("cs.transferNote")}</p>
          </section>

          <section id="template" aria-labelledby="template-h" className={`card ${s.section}`}>
            <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-start", gap: 16, flexWrap: "wrap" }}>
              <div><h2 id="template-h">{t("cs.reuse")}</h2><p className={s.lead}>{t("cs.reuseLead")}</p></div>
              <button type="button" className="btn btn-outline" onClick={() => { makeTemplate.reset(); setDialog("template"); }}>{t("cs.makeTemplate")}</button>
            </div>
            <div className={s.copy}>
              <div><p style={{ fontWeight: 600, marginBottom: 2 }}>{t("cs.copied")}</p><p className="muted">{plural(tree.data?.modules.length ?? 0, "section")}, {plural(items.filter((x) => !isAssignment(x)).length, "lecture")}, {plural(items.filter(isAssignment).length, "assignment")}, {t("cs.copiedMore")}</p></div>
              <div><p style={{ fontWeight: 600, marginBottom: 2 }}>{t("cs.notCopied")}</p><p className="muted">{t("cs.notCopiedList")}</p></div>
            </div>
          </section>

          <section id="archive" aria-labelledby="archive-h" className={`card ${s.section}`}>
            <h2 id="archive-h" style={{ marginBottom: 12 }}>{t("cs.archive")}</h2>
            <div className={s.danger}>
              <div style={{ flex: "1 1 320px" }}>
                <p style={{ fontWeight: 500 }}>{archived ? t("cs.restoreThis") : t("cs.archiveThis")}</p>
                <p className="muted" style={{ fontSize: 14 }}>{archived ? t("cs.restoreLead") : t("cs.archiveLead")}</p>
              </div>
              <button type="button" className="btn btn-outline" onClick={() => (archived ? void archive.run(false) : setDialog("archive"))}>{archived ? t("cs.restore") : t("cs.archiveBtn")}</button>
            </div>
            <div className={s.danger}>
              <div style={{ flex: "1 1 320px" }}>
                <p style={{ fontWeight: 500 }}>{t("cs.deleteThis")}</p>
                <p id="del-why" className="muted" style={{ fontSize: 14 }}>{i.has_grades ? t("cs.hasGrades") : t("cs.deleteLead")}</p>
              </div>
              <button type="button" className="btn btn-danger" disabled={i.has_grades} aria-describedby="del-why" onClick={() => { del.reset(); setDialog("delete"); }}>{t("cs.deleteBtn")}</button>
            </div>
          </section>
        </div>
      </div>

      {dialog === "group" && <LinkGroup courseId={courseId} onClose={close} />}
      {dialog === "owner" && (
        <Dialog title={t("cs.transfer")} description={t("cs.transferLead")} onClose={close}
          actions={<><button type="button" className="btn" onClick={close}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={!newOwner || transfer.pending} onClick={async () => { if (await transfer.run(newOwner)) close(); }}>{t("cs.transferBtn")}</button></>}>
          <div className="field">
            <label htmlFor="new-owner">{t("cs.newOwner")}</label>
            <select id="new-owner" className="select" value={newOwner} onChange={(e) => setNewOwner(e.target.value)}>
              <option value="">{t("cs.chooseTeacher")}</option>
              {teachers.map((x) => <option key={x.user_id} value={x.user_id}>{x.name} {x.surname}</option>)}
            </select>
            {teachers.length === 0 && <p className="hint">{t("cs.noTeacher")}</p>}
          </div>
          {transfer.error && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{transfer.error.errors[0]?.message ?? transfer.error.detail ?? transfer.error.title}</p>}
        </Dialog>
      )}
      {dialog === "archive" && (
        <ConfirmDialog title={t("cs.archiveQ")} confirm={t("cs.archiveBtn")} pending={archive.pending} error={archive.error?.detail} onClose={close} onConfirm={async () => { if (await archive.run(true)) close(); }}>
          {t("cs.archiveConfirm")}
        </ConfirmDialog>
      )}
      {dialog === "delete" && (
        <ConfirmDialog title={t("cs.deleteQ")} confirm={t("cs.deleteBtn")} danger pending={del.pending} error={del.error?.detail} onClose={close} onConfirm={async () => { if (await del.run()) router.replace("/courses"); }}>
          {t("cs.deleteConfirm")}
        </ConfirmDialog>
      )}
      {dialog === "template" && (
        <ConfirmDialog title={t("cs.templateQ")} confirm={t("cs.makeTemplate")} pending={makeTemplate.pending} error={makeTemplate.error?.detail} onClose={close} onConfirm={async () => { if (await makeTemplate.run()) router.push("/courses"); }}>
          {t("cs.templateConfirm")}
        </ConfirmDialog>
      )}
    </div>
  );
}
