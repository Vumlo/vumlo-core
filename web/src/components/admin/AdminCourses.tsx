"use client";

import { useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useAdminCourses, useAdminTermStats, useAdminUsers, useChangeOwner, useCreateTerm } from "@/lib/api/hooks/admin";
import { useTerms } from "@/lib/api/hooks/courses";
import type { AdminCourseRow } from "@/lib/api/gaps";
import { monthShort, plural } from "@/lib/format";
import s from "./Admin.module.css";

const range = (a: string, b: string) => {
  const f = (d: string, year: boolean) => { const [y = "", m = "1", day = "1"] = d.split("-"); return `${Number(day)} ${monthShort(Number(m))}${year ? ` ${y}` : ""}`; };
  return `${f(a, a.slice(0, 4) !== b.slice(0, 4))} to ${f(b, true)}`;
};

export function AdminCourses() {
  const terms = useTerms();
  const stats = useAdminTermStats();
  const courses = useAdminCourses();
  const teachers = useAdminUsers({ role: "teacher" });
  const admins = useAdminUsers({ role: "admin" });
  const createTerm = useCreateTerm();
  const change = useChangeOwner();
  const [term, setTerm] = useState<string>("");
  const [status, setStatus] = useState<"all" | "active" | "archived">("all");
  const [dialog, setDialog] = useState<null | "term" | AdminCourseRow>(null);
  const [form, setForm] = useState({ name: "", starts_on: "", ends_on: "" });
  const [owner, setOwner] = useState("");

  const activeTerm = term || stats.data?.find((x) => x.current)?.term_id || terms.data?.[0]?.id || "";
  const termName = terms.data?.find((t) => t.id === activeTerm)?.name ?? "";
  const rows = (courses.data ?? []).filter((c) => c.term_id === activeTerm && (status === "all" || (status === "archived" ? c.status === "archived" : c.status !== "archived")));
  const err = (f: string) => createTerm.error?.errors.find((e) => e.field === f)?.message;
  const candidates = [...(teachers.data ?? []), ...(admins.data ?? [])];

  return (
    <>
      <div className={s.head}><div><h1>Terms and courses</h1><p className="muted" style={{ marginTop: 4 }}>Teachers create their own courses. You set the terms and can step in when a course needs a new owner.</p></div></div>
      <section aria-labelledby="tm-h" className={`card ${s.section}`} style={{ marginBottom: 24 }}>
        <div className="row" style={{ justifyContent: "space-between", marginBottom: 12 }}>
          <h2 id="tm-h" style={{ fontSize: 18, fontWeight: 600 }}>Terms</h2>
          <button type="button" className="btn" onClick={() => { setForm({ name: "", starts_on: "", ends_on: "" }); createTerm.reset(); setDialog("term"); }}><Icon name="plus" size={15} strokeWidth={2.2} />Add a term</button>
        </div>
        <div className={s.terms}>
          {(terms.data ?? []).map((t) => {
            const st = stats.data?.find((x) => x.term_id === t.id);
            return (
              <div key={t.id} className={s.term} data-now={!!st?.current}>
                <p style={{ fontWeight: 600 }}>{t.name}{st?.current && <span className="pill pill-accent" style={{ marginLeft: 8, padding: "0 8px", fontSize: 12 }}>Now</span>}</p>
                <p className="muted" style={{ fontSize: 13 }}>{range(t.starts_on, t.ends_on)}</p>
                <p style={{ fontSize: 14, marginTop: 4 }}>{st ? `${plural(st.courses, "course")}${st.read_only ? ", read only" : ""}` : ""}</p>
              </div>
            );
          })}
        </div>
      </section>
      <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 12 }}>
        <h2 style={{ fontSize: 18, fontWeight: 600, flex: "1 1 auto" }}>Courses in {termName}</h2>
        <select aria-label="Term" className="select" style={{ width: "auto", minHeight: 40 }} value={activeTerm} onChange={(e) => setTerm(e.target.value)}>{(terms.data ?? []).map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}</select>
        <select aria-label="Status" className="select" style={{ width: "auto", minHeight: 40 }} value={status} onChange={(e) => setStatus(e.target.value as typeof status)}><option value="all">Active and archived</option><option value="active">Active</option><option value="archived">Archived</option></select>
      </div>
      <div className="card table-wrap">
        <table className="table">
          <thead><tr><th scope="col">Course</th><th scope="col">Owner</th><th scope="col" className="num">Students</th><th scope="col">Status</th><th scope="col"><span className="sr-only">Actions</span></th></tr></thead>
          <tbody>
            {rows.map((c) => (
              <tr key={c.course_id}>
                <th scope="row"><span className="tag" style={{ marginRight: 8 }}>{c.code}</span><span style={{ fontWeight: 500 }}>{c.title}</span></th>
                <td>{c.owner_name}</td>
                <td className="num">{c.student_count}</td>
                <td>{c.status === "archived" ? <span className="pill pill-dashed">Archived</span> : c.status === "draft" ? <span className="pill">Draft</span> : <span className="pill pill-ok">Active</span>}</td>
                <td><button type="button" className="btn" style={{ minHeight: 34 }} onClick={() => { setOwner(""); change.reset(); setDialog(c); }}>Change owner</button></td>
              </tr>
            ))}
          </tbody>
        </table>
        {courses.data && rows.length === 0 && <p className="empty">No courses match.</p>}
      </div>
      {dialog === "term" && (
        <Dialog title="Add a term" onClose={() => setDialog(null)} actions={<><button type="button" className="btn" onClick={() => setDialog(null)}>Cancel</button><button type="button" className="btn btn-primary" disabled={createTerm.pending} onClick={async () => { if (await createTerm.run(form)) setDialog(null); }}>Add term</button></>}>
          <div className="stack" style={{ gap: 12 }}>
            <div className="field"><label htmlFor="tn">Name</label><input id="tn" className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} aria-invalid={!!err("name")} />{err("name") && <p className="hint" style={{ color: "var(--danger)" }}>{err("name")}</p>}</div>
            <div className="row" style={{ gap: 12, alignItems: "flex-start" }}>
              <div className="field" style={{ flex: 1 }}><label htmlFor="ts">First day</label><input id="ts" className="input" type="date" value={form.starts_on} onChange={(e) => setForm({ ...form, starts_on: e.target.value })} aria-invalid={!!err("starts_on")} />{err("starts_on") && <p className="hint" style={{ color: "var(--danger)" }}>{err("starts_on")}</p>}</div>
              <div className="field" style={{ flex: 1 }}><label htmlFor="te">Last day</label><input id="te" className="input" type="date" value={form.ends_on} onChange={(e) => setForm({ ...form, ends_on: e.target.value })} aria-invalid={!!err("ends_on")} />{err("ends_on") && <p className="hint" style={{ color: "var(--danger)" }}>{err("ends_on")}</p>}</div>
            </div>
            {createTerm.error && !createTerm.error.errors.length && <p role="alert" className="alert alert-danger">{createTerm.error.detail ?? createTerm.error.title}</p>}
          </div>
        </Dialog>
      )}
      {dialog && dialog !== "term" && (
        <Dialog title={`Change the owner of ${dialog.code}`} description={`${dialog.title} is owned by ${dialog.owner_name}. The new owner can manage settings and the teaching team.`} onClose={() => setDialog(null)}
          actions={<><button type="button" className="btn" onClick={() => setDialog(null)}>Cancel</button><button type="button" className="btn btn-primary" disabled={!owner || change.pending} onClick={async () => { if (await change.run(dialog.course_id, owner)) setDialog(null); }}>Change owner</button></>}>
          <div className="field"><label htmlFor="own">New owner</label><select id="own" className="select" value={owner} onChange={(e) => setOwner(e.target.value)}><option value="">Choose a teacher</option>{candidates.filter((u) => u.id !== dialog.owner_id && u.status === "active").map((u) => <option key={u.id} value={u.id}>{u.name} {u.surname}</option>)}</select></div>
          {change.error && <p role="alert" className="alert alert-danger" style={{ marginTop: 10 }}>{change.error.errors[0]?.message ?? change.error.detail ?? change.error.title}</p>}
        </Dialog>
      )}
    </>
  );
}
