"use client";

import { useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useAddGroupMembers, useAdminUsers, useCreateGroup, useGroupOverview, useGroupOverviews, useGroups, useRemoveGroupMembers, useReplaceGroupMembers } from "@/lib/api/hooks/admin";
import { dayLong } from "@/lib/format";
import { plural } from "@/lib/format";
import { useT } from "@/lib/i18n";
import s from "./Admin.module.css";

function AddStudents({ groupId, onClose }: { groupId: string; onClose: () => void }) {
  const t = useT();
  const [q, setQ] = useState("");
  const users = useAdminUsers({ role: "student", email: q || undefined });
  const overview = useGroupOverview(groupId);
  const add = useAddGroupMembers(groupId);
  const [picked, setPicked] = useState<string[]>([]);
  const inGroup = new Set((overview.data?.members ?? []).map((m) => m.user_id));
  const list = (users.data ?? []).filter((u) => !inGroup.has(u.id));
  return (
    <Dialog title={t("ag.addStudents")} description={t("ag.addLead")} onClose={onClose}
      actions={<><button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={picked.length === 0 || add.pending} onClick={async () => { if (await add.run(picked)) onClose(); }}>{t("ag.addN", { n: picked.length || "" })}</button></>}>
      <div className="field"><label htmlFor="sq">{t("people.nameOrEmail")}</label><input id="sq" className="input" value={q} onChange={(e) => setQ(e.target.value)} /></div>
      <ul style={{ marginTop: 10, maxHeight: 260, overflow: "auto" }} aria-label={t("ag.canAdd")}>
        {list.map((u) => (
          <li key={u.id}><label className="check"><input type="checkbox" checked={picked.includes(u.id)} onChange={() => setPicked((p) => (p.includes(u.id) ? p.filter((x) => x !== u.id) : [...p, u.id]))} /><span>{u.name} {u.surname} <span className="muted" style={{ fontSize: 13 }}>{u.email}</span></span></label></li>
        ))}
        {list.length === 0 && <li className="muted" style={{ fontSize: 14 }}>{t("ag.nobody")}</li>}
      </ul>
      {add.error && <p role="alert" className="alert alert-danger" style={{ marginTop: 10 }}>{add.error.detail ?? add.error.title}</p>}
    </Dialog>
  );
}

function Replace({ groupId, onClose }: { groupId: string; onClose: () => void }) {
  const t = useT();
  const users = useAdminUsers({ role: "student" });
  const replace = useReplaceGroupMembers(groupId);
  const [text, setText] = useState("");
  const [done, setDone] = useState<{ added: number; removed: number; unknown: string[] } | null>(null);
  async function run() {
    const emails = text.split(/[\s,;]+/).map((x) => x.trim().toLowerCase()).filter(Boolean);
    const byEmail = new Map((users.data ?? []).map((u) => [u.email, u.id]));
    const ids = emails.map((e) => byEmail.get(e)).filter((x): x is string => !!x);
    const unknown = emails.filter((e) => !byEmail.has(e));
    const r = await replace.run(ids);
    if (r) setDone({ ...r, unknown });
  }
  return (
    <Dialog title={t("ag.replace")} description={t("ag.replaceLead")} onClose={onClose}
      actions={done ? <button type="button" className="btn btn-primary" onClick={onClose}>{t("common.done")}</button> : <><button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button><button type="button" className="btn btn-danger" disabled={replace.pending || !text.trim()} onClick={run}>{t("ag.replaceBtn")}</button></>}>
      {done ? (
        <div role="status" className="stack" style={{ gap: 8 }}><p>{t("ag.result", { added: done.added, removed: done.removed })}</p>{done.unknown.length > 0 && <p className="alert alert-danger">{t("ag.unknown", { list: done.unknown.join(", ") })}</p>}</div>
      ) : (
        <div className="field"><label htmlFor="emails">{t("ag.emails")}</label><textarea id="emails" className="textarea" style={{ minHeight: 160 }} value={text} onChange={(e) => setText(e.target.value)} /></div>
      )}
    </Dialog>
  );
}

export function AdminGroups() {
  const t = useT();
  const groups = useGroups();
  const ids = (groups.data ?? []).map((g) => g.id);
  const counts = useGroupOverviews(ids);
  const create = useCreateGroup();
  const [selId, setSelId] = useState<string | null>(null);
  const current = groups.data?.find((g) => g.id === selId) ?? groups.data?.[0];
  const overview = useGroupOverview(current?.id ?? null);
  const remove = useRemoveGroupMembers(current?.id ?? "");
  const [picked, setPicked] = useState<string[]>([]);
  const [dialog, setDialog] = useState<null | "new" | "add" | "replace">(null);
  const [name, setName] = useState("");
  const o = overview.data;

  return (
    <>
      <div className={s.head}>
        <div><h1>{t("ag.groups")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("ag.lead")}</p></div>
        <button type="button" className="btn btn-primary" onClick={() => { setName(""); create.reset(); setDialog("new"); }}><Icon name="plus" size={16} strokeWidth={2.2} />{t("ag.newGroup")}</button>
      </div>
      {!groups.data ? <p className="loading" role="status">{t("common.loading")}</p> : (
        <div className={s.split}>
          <ul aria-label={t("ag.groups")} className={`card ${s.groups}`}>
            {groups.data.map((g) => (
              <li key={g.id}><button type="button" className={s.groupLink} aria-current={g.id === current?.id} onClick={() => { setSelId(g.id); setPicked([]); }}><span>{g.name}</span><span className="muted" style={{ fontSize: 13, fontWeight: 400 }}>{counts.data?.[g.id]?.student_count ?? ""}</span></button></li>
            ))}
          </ul>
          <div className={s.groupMain}>
            {current && o && (
              <section aria-labelledby="gm-h" className={`card ${s.section}`}>
                <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 14 }}>
                  <div style={{ flex: "1 1 220px" }}><h2 id="gm-h" style={{ fontSize: 20, fontWeight: 600 }}>{current.name}</h2><p className="muted" style={{ fontSize: 14 }}>{plural(o.student_count, "student")}, {t("ag.linkedTo", { n: plural(o.course_count, "course") })}</p></div>
                  <button type="button" className="btn" onClick={() => setDialog("add")}>{t("ag.addStudents")}</button>
                  <button type="button" className="btn" onClick={() => setDialog("replace")}>{t("ag.replace")}</button>
                </div>
                {picked.length > 0 && (
                  <div className="alert alert-info" style={{ marginBottom: 12, alignItems: "center" }}>
                    <span style={{ flex: 1 }}>{t("ag.selected", { n: picked.length })}</span>
                    <button type="button" className="btn btn-danger" disabled={remove.pending} onClick={async () => { if (await remove.run(picked)) setPicked([]); }}>{t("ag.removeFrom")}</button>
                  </div>
                )}
                <div className="table-wrap">
                  <table className="table">
                    <thead><tr><th scope="col"><span className="sr-only">{t("ag.select")}</span></th><th scope="col">{t("grading.student")}</th><th scope="col">{t("ag.since")}</th></tr></thead>
                    <tbody>
                      {o.members.map((m) => (
                        <tr key={m.user_id} style={{ background: picked.includes(m.user_id) ? "var(--accent-soft)" : undefined }}>
                          <td style={{ width: 48 }}><input type="checkbox" style={{ width: 18, height: 18, accentColor: "var(--accent)" }} checked={picked.includes(m.user_id)} aria-label={t("ag.selectOne", { name: `${m.name} ${m.surname}` })} onChange={() => setPicked((p) => (p.includes(m.user_id) ? p.filter((x) => x !== m.user_id) : [...p, m.user_id]))} /></td>
                          <th scope="row">{m.name} {m.surname}<span className="muted" style={{ display: "block", fontSize: 13, fontWeight: 400 }}>{m.email}</span></th>
                          <td>{m.added_at ? dayLong(m.added_at) : "—"}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  {o.members.length === 0 && <p className="empty">{t("ag.empty")}</p>}
                </div>
                <p className="muted" style={{ marginTop: 10, fontSize: 13 }}>{t("ag.showing", { n: o.members.length, total: o.student_count })}</p>
              </section>
            )}
            {o && (
              <section aria-labelledby="gl-h" className={`card ${s.section}`}>
                <h2 id="gl-h" style={{ fontSize: 17, fontWeight: 600 }}>{t("ag.linkedCourses")}</h2>
                <p className="muted" style={{ margin: "4px 0 10px", fontSize: 14 }}>{t("ag.linkedLead")}</p>
                <ul>
                  {o.linked_courses.map((c) => <li key={c.course_id} style={{ padding: "10px 0", borderTop: "1px solid var(--border)" }}><span style={{ display: "block", fontWeight: 500 }}>{c.code} {c.title}</span><span className="muted" style={{ fontSize: 13 }}>{c.owner_name} {c.owner_surname}, {t("people.linkedOn", { day: dayLong(c.linked_on) })}</span></li>)}
                  {o.linked_courses.length === 0 && <li className="muted" style={{ fontSize: 14 }}>{t("ag.noCourse")}</li>}
                </ul>
              </section>
            )}
            <section aria-labelledby="gy-h" className={`card ${s.section}`}>
              <h2 id="gy-h" style={{ fontSize: 17, fontWeight: 600 }}>{t("ag.newYear")}</h2>
              <p className="muted" style={{ marginTop: 4, fontSize: 14 }}>{t("ag.newYearLead")}</p>
            </section>
          </div>
        </div>
      )}
      {dialog === "new" && (
        <Dialog title={t("ag.newGroup")} onClose={() => setDialog(null)} actions={<><button type="button" className="btn" onClick={() => setDialog(null)}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={create.pending} onClick={async () => { const g = await create.run(name); if (g) { setSelId(g.id); setDialog(null); } }}>{t("ag.create")}</button></>}>
          <div className="field"><label htmlFor="gname">{t("ag.name")}</label><input id="gname" className="input" value={name} onChange={(e) => setName(e.target.value)} aria-invalid={!!create.error} />{create.error && <p role="alert" className="hint" style={{ color: "var(--danger)" }}>{create.error.errors[0]?.message ?? create.error.detail ?? create.error.title}</p>}</div>
        </Dialog>
      )}
      {dialog === "add" && current && <AddStudents groupId={current.id} onClose={() => setDialog(null)} />}
      {dialog === "replace" && current && <Replace groupId={current.id} onClose={() => setDialog(null)} />}
    </>
  );
}
