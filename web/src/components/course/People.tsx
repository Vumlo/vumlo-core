"use client";

import { useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useCandidates, useCoursePeople, useGroupOptions, useLinkGroup, usePutMember, useRemoveMember, useUnlinkGroup } from "@/lib/api/hooks/courses";
import type { PersonRow } from "@/lib/api/gaps";
import { dayLong, initials, plural } from "@/lib/format";
import { useMe } from "@/lib/session";
import { CourseHeader } from "./CourseHeader";
import { useCourseAccess } from "./useCourseAccess";
import s from "./People.module.css";

const roleLabel = (p: PersonRow) => (p.is_owner ? "Owner, teacher" : p.role === "assistant" ? "Assistant" : "Teacher");

function AddPeople({ courseId, onClose }: { courseId: string; onClose: () => void }) {
  const [q, setQ] = useState("");
  const found = useCandidates(courseId, q);
  const put = usePutMember(courseId);
  const [role, setRole] = useState<"student" | "assistant" | "teacher">("student");
  return (
    <Dialog title="Add people" description="Find someone with an account at your institution." onClose={onClose} actions={<button type="button" className="btn btn-primary" onClick={onClose}>Done</button>}>
      <div className="stack" style={{ gap: 12 }}>
        <div className="field">
          <label htmlFor="find">Name or email</label>
          <input id="find" className="input" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Start typing" />
        </div>
        <div className="field">
          <label htmlFor="add-role">Add as</label>
          <select id="add-role" className="select" value={role} onChange={(e) => setRole(e.target.value as typeof role)}>
            <option value="student">Student</option>
            <option value="assistant">Assistant</option>
            <option value="teacher">Teacher</option>
          </select>
        </div>
        {put.error && <p role="alert" className="alert alert-danger">{put.error.detail ?? put.error.title}</p>}
        <ul aria-label="People you can add">
          {(found.data ?? []).map((c) => (
            <li key={c.user_id} className={s.found}>
              <span className={s.avatarSm}>{initials(c.name, c.surname)}</span>
              <span className="grow"><span style={{ display: "block", fontWeight: 500 }}>{c.name} {c.surname}</span><span className="muted" style={{ fontSize: 13 }}>{c.email}</span></span>
              <button type="button" className="btn" onClick={() => void put.run(c.user_id, role)}>Add</button>
            </li>
          ))}
          {found.data?.length === 0 && <li className="muted" style={{ padding: 8, fontSize: 14 }}>Nobody found, or everyone matching is already in the course.</li>}
        </ul>
      </div>
    </Dialog>
  );
}

export function LinkGroup({ courseId, onClose }: { courseId: string; onClose: () => void }) {
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
    <Dialog title="Link a group" description="Students of a linked group join and leave this course with the group." onClose={onClose}
      actions={<><button type="button" className="btn" onClick={onClose}>Cancel</button><button type="button" className="btn btn-primary" disabled={!groupId || link.pending} onClick={submit}>Link group</button></>}>
      <div className="stack" style={{ gap: 12 }}>
        <div className="field">
          <label htmlFor="grp">Group</label>
          <select id="grp" className="select" value={groupId} onChange={(e) => { setGroupId(e.target.value); setSubId(""); }}>
            <option value="">Choose a group</option>
            {(options.data ?? []).map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
          </select>
        </div>
        {group && group.subgroups.length > 0 && (
          <div className="field">
            <label htmlFor="sub">Subgroup</label>
            <select id="sub" className="select" value={subId} onChange={(e) => setSubId(e.target.value)}>
              <option value="">The whole group</option>
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
          <input type="search" aria-label="Find a person" placeholder="Find a person" value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
        <span className="muted" style={{ flex: "1 1 auto", fontSize: 14 }}>{people.data ? count : ""}</span>
        {staff && (
          <>
            <button type="button" className="btn btn-outline" onClick={() => setDialog("group")}>Link a group</button>
            <button type="button" className="btn btn-primary" onClick={() => setDialog("add")}>Add people</button>
          </>
        )}
      </div>
      {remove.error && <p role="alert" className="alert alert-danger" style={{ marginBottom: 16 }}>{remove.error.detail ?? remove.error.title}</p>}
      {!people.data ? <p className="loading" role="status">Loading…</p> : (
        <>
          <section aria-labelledby="staff-h">
            <h2 id="staff-h" className="section-title">Teaching</h2>
            <div className="card card-list">
              {teaching.map((p) => (
                <div key={p.user_id} className={s.row}>
                  <span aria-hidden="true" className={s.avatarLg}>{initials(p.name, p.surname)}</span>
                  <span className={s.who}>
                    <span style={{ display: "block", fontWeight: 500 }}>{p.role === "student" ? "" : "Dr. "}{p.name} {p.surname}</span>
                    {p.email && <a href={`mailto:${p.email}`} className={s.mail}>{p.email}</a>}
                  </span>
                  <span className={s.role}>{roleLabel(p)}</span>
                  {staff && !p.is_owner && (
                    <span style={{ position: "relative" }}>
                      <button type="button" className="icon-btn" style={{ width: 36, height: 36 }} aria-label={`More actions for ${p.name} ${p.surname}`} aria-haspopup="menu" aria-expanded={menu === p.user_id} onClick={() => setMenu(menu === p.user_id ? null : p.user_id)}>
                        <Icon name="more" size={18} strokeWidth={3.4} />
                      </button>
                      {menu === p.user_id && (
                        <div role="menu" className="popover" style={{ top: 38, right: 0 }}>
                          <button type="button" role="menuitem" className="menu-item" onClick={() => { void put.run(p.user_id, p.role === "assistant" ? "teacher" : "assistant"); setMenu(null); }}>
                            Make {p.role === "assistant" ? "teacher" : "assistant"}
                          </button>
                          <button type="button" role="menuitem" className="menu-item" style={{ color: "var(--danger)" }} onClick={() => { void remove.run(p.user_id); setMenu(null); }}>Remove from course</button>
                        </div>
                      )}
                    </span>
                  )}
                </div>
              ))}
              {teaching.length === 0 && <p className="empty">Nobody matches.</p>}
            </div>
          </section>

          {staff && (
            <section aria-labelledby="groups-h" style={{ marginTop: 28 }}>
              <h2 id="groups-h" className="section-title" style={{ marginBottom: 4 }}>Linked groups</h2>
              <p className="muted" style={{ margin: "0 2px 10px", fontSize: 14 }}>Students of a linked group join and leave this course with the group.</p>
              <div className={s.groups}>
                {people.data.groups.map((g) => (
                  <div key={g.link_id} className={`card ${s.group}`}>
                    <span className="grow">
                      <span style={{ display: "block", fontWeight: 600 }}>{g.name}</span>
                      <span className="muted" style={{ display: "block", fontSize: 13 }}>{plural(g.student_count, "student")}, linked {dayLong(g.linked_on)}</span>
                    </span>
                    <button type="button" className="btn btn-quiet" style={{ color: "var(--danger)", minHeight: 36, fontSize: 13 }} onClick={() => void unlink.run(g.link_id)}>Unlink</button>
                  </div>
                ))}
                {people.data.groups.length === 0 && <p className="muted" style={{ fontSize: 14 }}>No group is linked.</p>}
              </div>
            </section>
          )}

          <section aria-labelledby="students-h" style={{ marginTop: 28 }}>
            <h2 id="students-h" className="section-title">Students</h2>
            {!staff ? (
              <>
                <div className={s.grid}>
                  {students.map((p) => (
                    <div key={p.user_id} className={s.person}>
                      <span aria-hidden="true" className={s.avatarSm}>{initials(p.name, p.surname)}</span>
                      <span className="grow" style={{ fontWeight: 500 }}>{p.name} {p.surname}</span>
                      {p.user_id === me.data?.id && <span className={s.you}>You</span>}
                      <span className="muted" style={{ flex: "none", fontSize: 12 }}>{p.group ?? "Added directly"}</span>
                    </div>
                  ))}
                </div>
                <p className="muted" style={{ margin: "12px 2px 0", fontSize: 13 }}>Students see each other&apos;s names and groups. Email addresses are shown only for the teaching team.</p>
              </>
            ) : (
              <>
                <div className="card" style={{ overflowX: "auto" }}>
                  <div role="table" aria-label="Students" className={s.table}>
                    <div role="row" className={s.thead}>
                      <span role="columnheader" style={{ flex: "1 1 240px" }}>Name</span>
                      <span role="columnheader" style={{ flex: "1 1 240px" }}>Email</span>
                      <span role="columnheader" style={{ flex: "0 0 170px" }}>How they joined</span>
                      <span role="columnheader" style={{ flex: "0 0 100px", textAlign: "right" }}>Actions</span>
                    </div>
                    {students.map((p) => (
                      <div key={p.user_id} role="row" className={s.trow}>
                        <span role="cell" className={s.c1}><span aria-hidden="true" className={s.avatarSm} style={{ width: 30, height: 30 }}>{initials(p.name, p.surname)}</span><span style={{ fontWeight: 500 }}>{p.name} {p.surname}</span></span>
                        <span role="cell" className={s.c2}>{p.email}</span>
                        <span role="cell" className={s.c3}>{p.source === "group" ? `Group ${p.group}` : "Added directly"}</span>
                        <span role="cell" className={s.c4}>
                          {p.source === "direct" ? (
                            <button type="button" className="btn btn-quiet" style={{ color: "var(--danger)", minHeight: 34, fontSize: 13 }} onClick={() => void remove.run(p.user_id)}>Remove</button>
                          ) : <span title="Remove them from the group instead" className="muted" style={{ fontSize: 13 }}>Via group</span>}
                        </span>
                      </div>
                    ))}
                    {students.length === 0 && <p className="empty">No students match.</p>}
                  </div>
                </div>
                {former.length > 0 && (
                  <>
                    <h3 style={{ margin: "24px 2px 4px", fontSize: 16, fontWeight: 600 }}>Former students</h3>
                    <p className="muted" style={{ margin: "0 2px 10px", fontSize: 14 }}>They keep their published grades and can&apos;t be graded again.</p>
                    {former.map((p) => (
                      <div key={p.user_id} className="card row" style={{ gap: 12, minHeight: 56, padding: "8px 18px" }}>
                        <span aria-hidden="true" className={s.avatarSm} style={{ width: 30, height: 30 }}>{initials(p.name, p.surname)}</span>
                        <span className="grow muted">{p.name} {p.surname}</span>
                        <span className="muted" style={{ fontSize: 13 }}>Left on {p.left_on ? dayLong(p.left_on) : ""}</span>
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
