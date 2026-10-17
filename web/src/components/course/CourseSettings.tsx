"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useInstitution } from "@/lib/api/hooks/institution";
import { useArchiveCourse, useContentTree, useCourse, useCourseInfo, useCoursePeople, useDeleteCourse, useMakeTemplate, useTerms, useTransferOwner, useUnlinkGroup, useUpdateCourse, useUpdateCourseInfo } from "@/lib/api/hooks/courses";
import { allItems, isAssignment } from "@/lib/course";
import { dayLabel, dayLong, initials, plural } from "@/lib/format";
import { CourseHeader } from "./CourseHeader";
import { LinkGroup } from "./People";
import s from "./Settings.module.css";

const NAV = [["general", "General"], ["grading", "Grading"], ["groups", "Student groups"], ["team", "Owner"], ["template", "Reuse next term"], ["archive", "Archive or delete"]] as const;

export function CourseSettings({ courseId }: { courseId: string }) {
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
  if (!c || !i) return <div className="container"><p className="loading" role="status">Loading…</p></div>;

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
        <nav aria-label="Settings" className={s.nav}>
          {NAV.map(([id, label]) => <a key={id} href={`#${id}`} aria-current={active === id} onClick={() => setActive(id)}>{label}</a>)}
        </nav>
        <div className={s.main}>
          <section id="general" aria-labelledby="general-h" className={`card ${s.section}`}>
            <h2 id="general-h">General</h2>
            <p className={s.lead}>Students see these on the course card and at the top of the course.</p>
            <div className={s.fields}>
              <div className="field">
                <label htmlFor="f-title">Course title</label>
                <input id="f-title" className="input" value={tTitle} onChange={(e) => setTitle(e.target.value)} aria-invalid={!!fieldError("title")} />
                {fieldError("title") && <p className="hint" style={{ color: "var(--danger)" }}>{fieldError("title")}</p>}
              </div>
              <div className={s.two}>
                <div className="field"><label htmlFor="f-code">Course code</label><input id="f-code" className="input" value={tCode} onChange={(e) => setCode(e.target.value)} /></div>
                <div className="field">
                  <label htmlFor="f-term">Term</label>
                  <select id="f-term" className="select" value={tTerm} onChange={(e) => setTermId(e.target.value)}>
                    <option value="">No term</option>
                    {(terms.data ?? []).map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
                  </select>
                </div>
              </div>
              <div className="field">
                <label htmlFor="f-desc">Short description</label>
                <textarea id="f-desc" className="textarea" rows={3} value={tDesc} onChange={(e) => setDesc(e.target.value)} aria-invalid={!!fieldError("description")} />
                <p className="hint" style={{ color: tDesc.length > 300 ? "var(--danger)" : undefined }}>{tDesc.length} of 300 characters</p>
                {fieldError("description") && <p className="hint" style={{ color: "var(--danger)" }}>{fieldError("description")}</p>}
              </div>
            </div>
            <div className="row" style={{ gap: 14, marginTop: 18, flexWrap: "wrap" }}>
              <button type="button" className="btn btn-primary" disabled={!dirty || updateCourse.pending || updateInfo.pending} onClick={save}>Save changes</button>
              <span role="status" className="muted" style={{ fontSize: 14 }}>{saved ? "Saved." : i.settings_saved_at ? `Last saved ${dayLabel(i.settings_saved_at)} by you` : ""}</span>
            </div>
          </section>

          <section id="grading" aria-labelledby="grading-h" className={`card ${s.section}`}>
            <h2 id="grading-h">Grading</h2>
            <p className={s.lead}>The grade scale comes from the university, so it&apos;s the same in every course.</p>
            <dl className={s.dl}>
              <dt>Scale</dt><dd>{institution.data?.grading_scale ?? "…"}</dd>
              <dt>Course total</dt><dd>{i.total_points} points across {plural(i.graded_items, "graded item")}</dd>
              <dt>Grades students see</dt><dd>Only grades you publish. Drafts stay with the teaching team.</dd>
            </dl>
            <p className="muted" style={{ marginTop: 16, fontSize: 14 }}>To change the scale, contact your university&apos;s LMS administrator.</p>
          </section>

          <section id="groups" aria-labelledby="groups-h" className={`card ${s.section}`}>
            <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-start", gap: 16, flexWrap: "wrap" }}>
              <div><h2 id="groups-h">Student groups</h2><p className={s.lead}>Students from linked groups join the course on their own. When someone leaves a group, they leave the course too.</p></div>
              <button type="button" className="btn btn-outline" onClick={() => setDialog("group")}><Icon name="plus" size={15} strokeWidth={2.2} />Link a group</button>
            </div>
            <ul className={s.list}>
              {groups.map((g) => (
                <li key={g.link_id} className={s.li}>
                  <span className="grow"><span style={{ display: "block", fontWeight: 500 }}>{g.name}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>{plural(g.student_count, "student")}, linked {dayLong(g.linked_on)}</span></span>
                  <button type="button" className="btn btn-quiet" style={{ color: "var(--danger)" }} onClick={() => void unlink.run(g.link_id)}>Unlink</button>
                </li>
              ))}
              {groups.length === 0 && <li className="muted" style={{ fontSize: 14 }}>No group is linked.</li>}
            </ul>
          </section>

          <section id="team" aria-labelledby="team-h" className={`card ${s.section}`}>
            <h2 id="team-h">Owner</h2>
            <p className={s.lead}>The owner can change settings, archive the course and manage the teaching team. Add assistants on the <Link href={`/courses/${courseId}/people`} style={{ color: "var(--accent-fg)" }}>People tab</Link>.</p>
            {owner && (
              <div className={s.owner}>
                <span aria-hidden="true" className={s.avatar}>{initials(owner.name, owner.surname)}</span>
                <span className="grow"><span style={{ display: "block", fontWeight: 500 }}>{i.owner_name}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>Owner since {dayLong(c.created_at)}</span></span>
                <button type="button" className="btn btn-outline" onClick={() => { setNewOwner(""); transfer.reset(); setDialog("owner"); }}>Transfer ownership</button>
              </div>
            )}
            <p className="muted" style={{ fontSize: 14 }}>You can only pick someone who already teaches this course. You&apos;ll stay on as a teacher.</p>
          </section>

          <section id="template" aria-labelledby="template-h" className={`card ${s.section}`}>
            <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-start", gap: 16, flexWrap: "wrap" }}>
              <div><h2 id="template-h">Reuse next term</h2><p className={s.lead}>Make a template from this course. It&apos;s a full copy, so later changes here won&apos;t reach the template.</p></div>
              <button type="button" className="btn btn-outline" onClick={() => { makeTemplate.reset(); setDialog("template"); }}>Make a template</button>
            </div>
            <div className={s.copy}>
              <div><p style={{ fontWeight: 600, marginBottom: 2 }}>Copied</p><p className="muted">{plural(tree.data?.modules.length ?? 0, "section")}, {plural(items.filter((x) => !isAssignment(x)).length, "lecture")}, {plural(items.filter(isAssignment).length, "assignment")}, quiz questions and form fields</p></div>
              <div><p style={{ fontWeight: 600, marginBottom: 2 }}>Not copied</p><p className="muted">Students, groups, submissions, grades and due dates</p></div>
            </div>
          </section>

          <section id="archive" aria-labelledby="archive-h" className={`card ${s.section}`}>
            <h2 id="archive-h" style={{ marginBottom: 12 }}>Archive or delete</h2>
            <div className={s.danger}>
              <div style={{ flex: "1 1 320px" }}>
                <p style={{ fontWeight: 500 }}>{archived ? "Restore this course" : "Archive this course"}</p>
                <p className="muted" style={{ fontSize: 14 }}>{archived ? "The course opens again for submissions, grading and editing." : "Everyone can still open it and read their grades. Nobody can submit, grade or edit. You can restore it later."}</p>
              </div>
              <button type="button" className="btn btn-outline" onClick={() => (archived ? void archive.run(false) : setDialog("archive"))}>{archived ? "Restore course" : "Archive course"}</button>
            </div>
            <div className={s.danger}>
              <div style={{ flex: "1 1 320px" }}>
                <p style={{ fontWeight: 500 }}>Delete this course</p>
                <p id="del-why" className="muted" style={{ fontSize: 14 }}>{i.has_grades ? "This course already has grades, so it can’t be deleted. Archive it instead." : "This deletes the course and everything in it. You can’t undo it."}</p>
              </div>
              <button type="button" className="btn btn-danger" disabled={i.has_grades} aria-describedby="del-why" onClick={() => { del.reset(); setDialog("delete"); }}>Delete course</button>
            </div>
          </section>
        </div>
      </div>

      {dialog === "group" && <LinkGroup courseId={courseId} onClose={close} />}
      {dialog === "owner" && (
        <Dialog title="Transfer ownership" description="The new owner can change settings and manage the teaching team. You stay on as a teacher." onClose={close}
          actions={<><button type="button" className="btn" onClick={close}>Cancel</button><button type="button" className="btn btn-primary" disabled={!newOwner || transfer.pending} onClick={async () => { if (await transfer.run(newOwner)) close(); }}>Transfer</button></>}>
          <div className="field">
            <label htmlFor="new-owner">New owner</label>
            <select id="new-owner" className="select" value={newOwner} onChange={(e) => setNewOwner(e.target.value)}>
              <option value="">Choose a teacher</option>
              {teachers.map((t) => <option key={t.user_id} value={t.user_id}>{t.name} {t.surname}</option>)}
            </select>
            {teachers.length === 0 && <p className="hint">No other teacher is on this course yet. Add one on the People tab.</p>}
          </div>
          {transfer.error && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{transfer.error.errors[0]?.message ?? transfer.error.detail ?? transfer.error.title}</p>}
        </Dialog>
      )}
      {dialog === "archive" && (
        <ConfirmDialog title="Archive this course?" confirm="Archive course" pending={archive.pending} error={archive.error?.detail} onClose={close} onConfirm={async () => { if (await archive.run(true)) close(); }}>
          Everyone can still open it and read their grades. Nobody can submit, grade or edit until you restore it.
        </ConfirmDialog>
      )}
      {dialog === "delete" && (
        <ConfirmDialog title="Delete this course?" confirm="Delete course" danger pending={del.pending} error={del.error?.detail} onClose={close} onConfirm={async () => { if (await del.run()) router.replace("/courses"); }}>
          The course and everything in it is deleted for good.
        </ConfirmDialog>
      )}
      {dialog === "template" && (
        <ConfirmDialog title="Make a template?" confirm="Make a template" pending={makeTemplate.pending} error={makeTemplate.error?.detail} onClose={close} onConfirm={async () => { if (await makeTemplate.run()) router.push("/courses"); }}>
          The template is a full copy of this course&apos;s content, as drafts. Later changes here won&apos;t reach it.
        </ConfirmDialog>
      )}
    </div>
  );
}
