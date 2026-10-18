"use client";

import { useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useCandidates, useCoursePeople, useGroupOptions, useLinkGroup, usePutMember, useRemoveMember, useUnlinkGroup } from "@/lib/api/hooks/courses";
import type { PersonRow } from "@/lib/api/types";
import { linkedGroupName } from "@/lib/course";
import { dayLong, initials, plural } from "@/lib/format";
import { useMe } from "@/lib/session";
import { CourseHeader } from "./CourseHeader";
import { useCourseAccess } from "./useCourseAccess";
import { translate } from "@/lib/i18n/core";
import { useT } from "@/lib/i18n";
import s from "./People.module.css";

const roleLabel = (p: PersonRow) => translate(p.is_owner ? "people.ownerTeacher" : p.role === "assistant" ? "role.assistant" : "role.teacher");

function AddPeople({ courseId, onClose }: { courseId: string; onClose: () => void }) {
  const t = useT();
  const [q, setQ] = useState("");
  const found = useCandidates(courseId, q);
  const put = usePutMember(courseId);
  const [role, setRole] = useState<"student" | "assistant" | "teacher">("student");
  return (
    <Dialog title={t("people.add")} description={t("people.addLead")} onClose={onClose} actions={<button type="button" className="btn btn-primary" onClick={onClose}>{t("common.done")}</button>}>
      <div className="stack" style={{ gap: 12 }}>
        <div className="field">
          <label htmlFor="find">{t("people.nameOrEmail")}</label>
          <input id="find" className="input" value={q} onChange={(e) => setQ(e.target.value)} placeholder={t("people.startTyping")} />
        </div>
        <div className="field">
          <label htmlFor="add-role">{t("people.addAs")}</label>
          <select id="add-role" className="select" value={role} onChange={(e) => setRole(e.target.value as typeof role)}>
            <option value="student">{t("role.student")}</option>
            <option value="assistant">{t("role.assistant")}</option>
            <option value="teacher">{t("role.teacher")}</option>
          </select>
        </div>
        {put.error && <p role="alert" className="alert alert-danger">{put.error.detail ?? put.error.title}</p>}
        <ul aria-label={t("people.canAdd")}>
          {(found.data ?? []).map((c) => (
            <li key={c.user_id} className={s.found}>
              <span className={s.avatarSm}>{initials(c.name, c.surname)}</span>
              <span className="grow"><span style={{ display: "block", fontWeight: 500 }}>{c.name} {c.surname}</span><span className="muted" style={{ fontSize: 13 }}>{c.email}</span></span>
              <button type="button" className="btn" onClick={() => void put.run(c.user_id, role)}>{t("people.addOne")}</button>
            </li>
          ))}
          {found.data?.length === 0 && <li className="muted" style={{ padding: 8, fontSize: 14 }}>{t("people.nobody")}</li>}
        </ul>
      </div>
    </Dialog>
  );
}

export function LinkGroup({ courseId, onClose }: { courseId: string; onClose: () => void }) {
  const t = useT();
  const options = useGroupOptions(courseId);
  const link = useLinkGroup(courseId);
  const [groupId, setGroupId] = useState("");
  const [subId, setSubId] = useState("");
  const group = options.data?.find((g) => g.id === groupId);
  async function submit() {
    const l = await link.run(groupId, subId || null);
    if (l) onClose();
  }
  return (
    <Dialog title={t("people.linkGroup")} description={t("people.linkedLead")} onClose={onClose}
      actions={<><button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={!groupId || link.pending} onClick={submit}>{t("people.linkGroupBtn")}</button></>}>
      <div className="stack" style={{ gap: 12 }}>
        <div className="field">
          <label htmlFor="grp">{t("people.group")}</label>
          <select id="grp" className="select" value={groupId} onChange={(e) => { setGroupId(e.target.value); setSubId(""); }}>
            <option value="">{t("people.chooseGroup")}</option>
            {(options.data ?? []).map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
          </select>
        </div>
        {group && group.subgroups.length > 0 && (
          <div className="field">
            <label htmlFor="sub">{t("people.subgroup")}</label>
            <select id="sub" className="select" value={subId} onChange={(e) => setSubId(e.target.value)}>
              <option value="">{t("people.wholeGroup")}</option>
              {group.subgroups.map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
            </select>
          </div>
        )}
        {link.error && <p role="alert" className="alert alert-danger">{link.error.detail ?? link.error.title}</p>}
      </div>
    </Dialog>
  );
}

export function People({ courseId }: { courseId: string }) {
  const t = useT();
  const me = useMe();
  const { staff } = useCourseAccess(courseId);
  const people = useCoursePeople(courseId);
  const remove = useRemoveMember(courseId);
  const unlink = useUnlinkGroup(courseId);
  const put = usePutMember(courseId);
  const [q, setQ] = useState("");
  const [dialog, setDialog] = useState<null | "add" | "group">(null);
  const [menu, setMenu] = useState<string | null>(null);

  const match = (p: PersonRow) => `${p.name} ${p.surname} ${p.email ?? ""}`.toLowerCase().includes(q.trim().toLowerCase());
  const all = people.data?.people ?? [];
  const teaching = all.filter((p) => p.role !== "student" && match(p));
  const students = all.filter((p) => p.role === "student" && !p.former && match(p));
  const former = all.filter((p) => p.role === "student" && p.former && match(p));
  const count = `${plural(all.filter((p) => p.role !== "student").length, "teaching", "teaching")}, ${plural(all.filter((p) => p.role === "student" && !p.former).length, "student")}${staff && all.some((p) => p.former) ? `, ${plural(all.filter((p) => p.former).length, "former student")}` : ""}`;

  return (
    <div className="container">
      <CourseHeader courseId={courseId} />
      <div className={s.bar}>
        <label className="search" style={{ flex: "0 1 280px", background: "var(--surface)" }}>
          <Icon name="search" size={16} />
          <input type="search" aria-label={t("people.find")} placeholder={t("people.find")} value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
        <span className="muted" style={{ flex: "1 1 auto", fontSize: 14 }}>{people.data ? count : ""}</span>
        {staff && (
          <>
            <button type="button" className="btn btn-outline" onClick={() => setDialog("group")}>{t("people.linkGroup")}</button>
            <button type="button" className="btn btn-primary" onClick={() => setDialog("add")}>{t("people.add")}</button>
          </>
        )}
      </div>
      {remove.error && <p role="alert" className="alert alert-danger" style={{ marginBottom: 16 }}>{remove.error.detail ?? remove.error.title}</p>}
      {!people.data ? <p className="loading" role="status">{t("common.loading")}</p> : (
        <>
          <section aria-labelledby="staff-h">
            <h2 id="staff-h" className="section-title">{t("people.teaching")}</h2>
            <div className="card card-list">
              {teaching.map((p) => (
                <div key={p.user_id} className={s.row}>
                  <span aria-hidden="true" className={s.avatarLg}>{initials(p.name, p.surname)}</span>
                  <span className={s.who}>
                    <span style={{ display: "block", fontWeight: 500 }}>{p.role === "student" ? "" : t("people.doctor")}{p.name} {p.surname}</span>
                    {p.email && <a href={`mailto:${p.email}`} className={s.mail}>{p.email}</a>}
                  </span>
                  <span className={s.role}>{roleLabel(p)}</span>
                  {staff && !p.is_owner && (
                    <span style={{ position: "relative" }}>
                      <button type="button" className="icon-btn" style={{ width: 36, height: 36 }} aria-label={t("people.moreFor", { name: `${p.name} ${p.surname}` })} aria-haspopup="menu" aria-expanded={menu === p.user_id} onClick={() => setMenu(menu === p.user_id ? null : p.user_id)}>
                        <Icon name="more" size={18} strokeWidth={3.4} />
                      </button>
                      {menu === p.user_id && (
                        <div role="menu" className="popover" style={{ top: 38, right: 0 }}>
                          <button type="button" role="menuitem" className="menu-item" onClick={() => { void put.run(p.user_id, p.role === "assistant" ? "teacher" : "assistant"); setMenu(null); }}>
                            {t(p.role === "assistant" ? "people.makeTeacher" : "people.makeAssistant")}
                          </button>
                          <button type="button" role="menuitem" className="menu-item" style={{ color: "var(--danger)" }} onClick={() => { void remove.run(p.user_id); setMenu(null); }}>{t("people.removeFromCourse")}</button>
                        </div>
                      )}
                    </span>
                  )}
                </div>
              ))}
              {teaching.length === 0 && <p className="empty">{t("people.nobodyMatches")}</p>}
            </div>
          </section>

          {staff && (
            <section aria-labelledby="groups-h" style={{ marginTop: 28 }}>
              <h2 id="groups-h" className="section-title" style={{ marginBottom: 4 }}>{t("people.linkedGroups")}</h2>
              <p className="muted" style={{ margin: "0 2px 10px", fontSize: 14 }}>{t("people.linkedLead")}</p>
              <div className={s.groups}>
                {people.data.groups.map((g) => (
                  <div key={g.link_id} className={`card ${s.group}`}>
                    <span className="grow">
                      <span style={{ display: "block", fontWeight: 600 }}>{linkedGroupName(g)}</span>
                      <span className="muted" style={{ display: "block", fontSize: 13 }}>{plural(g.student_count, "student")}, {t("people.linkedOn", { day: dayLong(g.linked_at) })}</span>
                    </span>
                    <button type="button" className="btn btn-quiet" style={{ color: "var(--danger)", minHeight: 36, fontSize: 13 }} onClick={() => void unlink.run(g.link_id)}>{t("people.unlink")}</button>
                  </div>
                ))}
                {people.data.groups.length === 0 && <p className="muted" style={{ fontSize: 14 }}>{t("people.noGroup")}</p>}
              </div>
            </section>
          )}

          <section aria-labelledby="students-h" style={{ marginTop: 28 }}>
            <h2 id="students-h" className="section-title">{t("people.students")}</h2>
            {!staff ? (
              <>
                <div className={s.grid}>
                  {students.map((p) => (
                    <div key={p.user_id} className={s.person}>
                      <span aria-hidden="true" className={s.avatarSm}>{initials(p.name, p.surname)}</span>
                      <span className="grow" style={{ fontWeight: 500 }}>{p.name} {p.surname}</span>
                      {p.user_id === me.data?.id && <span className={s.you}>{t("people.you")}</span>}
                      <span className="muted" style={{ flex: "none", fontSize: 12 }}>{p.group ?? t("people.direct")}</span>
                    </div>
                  ))}
                </div>
                <p className="muted" style={{ margin: "12px 2px 0", fontSize: 13 }}>{t("people.privacy")}</p>
              </>
            ) : (
              <>
                <div className="card" style={{ overflowX: "auto" }}>
                  <div role="table" aria-label={t("people.students")} className={s.table}>
                    <div role="row" className={s.thead}>
                      <span role="columnheader" style={{ flex: "1 1 240px" }}>{t("field.name")}</span>
                      <span role="columnheader" style={{ flex: "1 1 240px" }}>{t("field.email")}</span>
                      <span role="columnheader" style={{ flex: "0 0 170px" }}>{t("people.howJoined")}</span>
                      <span role="columnheader" style={{ flex: "0 0 100px", textAlign: "right" }}>{t("people.actions")}</span>
                    </div>
                    {students.map((p) => (
                      <div key={p.user_id} role="row" className={s.trow}>
                        <span role="cell" className={s.c1}><span aria-hidden="true" className={s.avatarSm} style={{ width: 30, height: 30 }}>{initials(p.name, p.surname)}</span><span style={{ fontWeight: 500 }}>{p.name} {p.surname}</span></span>
                        <span role="cell" className={s.c2}>{p.email}</span>
                        <span role="cell" className={s.c3}>{p.source === "group" ? t("people.group2", { name: p.group ?? "" }) : t("people.direct")}</span>
                        <span role="cell" className={s.c4}>
                          {p.source === "direct" ? (
                            <button type="button" className="btn btn-quiet" style={{ color: "var(--danger)", minHeight: 34, fontSize: 13 }} onClick={() => void remove.run(p.user_id)}>{t("people.remove")}</button>
                          ) : <span title={t("people.viaGroupHint")} className="muted" style={{ fontSize: 13 }}>{t("people.viaGroup")}</span>}
                        </span>
                      </div>
                    ))}
                    {students.length === 0 && <p className="empty">{t("people.noStudents")}</p>}
                  </div>
                </div>
                {former.length > 0 && (
                  <>
                    <h3 style={{ margin: "24px 2px 4px", fontSize: 16, fontWeight: 600 }}>{t("people.former")}</h3>
                    <p className="muted" style={{ margin: "0 2px 10px", fontSize: 14 }}>{t("people.formerLead")}</p>
                    {former.map((p) => (
                      <div key={p.user_id} className="card row" style={{ gap: 12, minHeight: 56, padding: "8px 18px" }}>
                        <span aria-hidden="true" className={s.avatarSm} style={{ width: 30, height: 30 }}>{initials(p.name, p.surname)}</span>
                        <span className="grow muted">{p.name} {p.surname}</span>
                        <span className="muted" style={{ fontSize: 13 }}>{t("people.left")}</span>
                      </div>
                    ))}
                  </>
                )}
              </>
            )}
          </section>
        </>
      )}
      {dialog === "add" && <AddPeople courseId={courseId} onClose={() => setDialog(null)} />}
      {dialog === "group" && <LinkGroup courseId={courseId} onClose={() => setDialog(null)} />}
    </div>
  );
}
