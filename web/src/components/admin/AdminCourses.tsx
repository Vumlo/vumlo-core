"use client";

import { useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useAdminCourses, useAdminTermStats, useAdminUsers, useChangeOwner, useCreateTerm } from "@/lib/api/hooks/admin";
import { useTerms } from "@/lib/api/hooks/courses";
import type { AdminCourse } from "@/lib/api/types";
import { monthShort, plural } from "@/lib/format";
import { translate } from "@/lib/i18n/core";
import { useT } from "@/lib/i18n";
import s from "./Admin.module.css";

const range = (a: string, b: string) => {
  const f = (d: string, year: boolean) => { const [y = "", m = "1", day = "1"] = d.split("-"); return `${Number(day)} ${monthShort(Number(m))}${year ? ` ${y}` : ""}`; };
  return translate("ac.range", { from: f(a, a.slice(0, 4) !== b.slice(0, 4)), to: f(b, true) });
};

export function AdminCourses() {
  const t = useT();
  const terms = useTerms();
  const stats = useAdminTermStats();
  const courses = useAdminCourses();
  const teachers = useAdminUsers({ role: "teacher" });
  const admins = useAdminUsers({ role: "admin" });
  const createTerm = useCreateTerm();
  const change = useChangeOwner();
  const [term, setTerm] = useState<string>("");
  const [status, setStatus] = useState<"all" | "active" | "archived">("all");
  const [dialog, setDialog] = useState<null | "term" | AdminCourse>(null);
  const [form, setForm] = useState({ name: "", starts_on: "", ends_on: "" });
  const [owner, setOwner] = useState("");

  const activeTerm = term || stats.data?.find((x) => x.current)?.term_id || terms.data?.[0]?.id || "";
  const termName = terms.data?.find((x) => x.id === activeTerm)?.name ?? "";
  const rows = (courses.data ?? []).filter((c) => c.term_id === activeTerm && (status === "all" || (status === "archived" ? c.status === "archived" : c.status !== "archived")));
  const err = (f: string) => createTerm.error?.errors.find((e) => e.field === f)?.message;
  const candidates = [...(teachers.data ?? []), ...(admins.data ?? [])];

  return (
    <>
      <div className={s.head}><div><h1>{t("ac.title")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("ac.lead")}</p></div></div>
      <section aria-labelledby="tm-h" className={`card ${s.section}`} style={{ marginBottom: 24 }}>
        <div className="row" style={{ justifyContent: "space-between", marginBottom: 12 }}>
          <h2 id="tm-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("ac.terms")}</h2>
          <button type="button" className="btn" onClick={() => { setForm({ name: "", starts_on: "", ends_on: "" }); createTerm.reset(); setDialog("term"); }}><Icon name="plus" size={15} strokeWidth={2.2} />{t("ac.addTerm")}</button>
        </div>
        <div className={s.terms}>
          {(terms.data ?? []).map((tm) => {
            const st = stats.data?.find((x) => x.term_id === tm.id);
            return (
              <div key={tm.id} className={s.term} data-now={!!st?.current}>
                <p style={{ fontWeight: 600 }}>{tm.name}{st?.current && <span className="pill pill-accent" style={{ marginLeft: 8, padding: "0 8px", fontSize: 12 }}>{t("ac.now")}</span>}</p>
                <p className="muted" style={{ fontSize: 13 }}>{range(tm.starts_on, tm.ends_on)}</p>
                <p style={{ fontSize: 14, marginTop: 4 }}>{st ? `${plural(st.courses, "course")}${st.read_only ? `, ${t("ac.readOnly")}` : ""}` : ""}</p>
              </div>
            );
          })}
        </div>
      </section>
      <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 12 }}>
        <h2 style={{ fontSize: 18, fontWeight: 600, flex: "1 1 auto" }}>{t("ac.coursesIn", { term: termName })}</h2>
        <select aria-label={t("newCourse.term")} className="select" style={{ width: "auto", minHeight: 40 }} value={activeTerm} onChange={(e) => setTerm(e.target.value)}>{(terms.data ?? []).map((tm) => <option key={tm.id} value={tm.id}>{tm.name}</option>)}</select>
        <select aria-label={t("ap.status")} className="select" style={{ width: "auto", minHeight: 40 }} value={status} onChange={(e) => setStatus(e.target.value as typeof status)}><option value="all">{t("ac.activeAndArchived")}</option><option value="active">{t("ap.active")}</option><option value="archived">{t("courses.archived")}</option></select>
      </div>
      <div className="card table-wrap">
        <table className="table">
          <thead><tr><th scope="col">{t("search.kind.course")}</th><th scope="col">{t("cs.owner")}</th><th scope="col" className="num">{t("people.students")}</th><th scope="col">{t("ap.status")}</th><th scope="col"><span className="sr-only">{t("people.actions")}</span></th></tr></thead>
          <tbody>
            {rows.map((c) => (
              <tr key={c.course_id}>
                <th scope="row"><span className="tag" style={{ marginRight: 8 }}>{c.code}</span><span style={{ fontWeight: 500 }}>{c.title}</span></th>
                <td>{c.owner_name} {c.owner_surname}</td>
                <td className="num">{c.student_count}</td>
                <td>{c.status === "archived" ? <span className="pill pill-dashed">{t("courses.archived")}</span> : c.status === "draft" ? <span className="pill">{t("common.draft")}</span> : <span className="pill pill-ok">{t("ap.active")}</span>}</td>
                <td><button type="button" className="btn" style={{ minHeight: 34 }} onClick={() => { setOwner(""); change.reset(); setDialog(c); }}>{t("ac.changeOwner")}</button></td>
              </tr>
            ))}
          </tbody>
        </table>
        {courses.data && rows.length === 0 && <p className="empty">{t("ac.none")}</p>}
      </div>
      {dialog === "term" && (
        <Dialog title={t("ac.addTerm")} onClose={() => setDialog(null)} actions={<><button type="button" className="btn" onClick={() => setDialog(null)}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={createTerm.pending} onClick={async () => { if (await createTerm.run(form)) setDialog(null); }}>{t("ac.addTermBtn")}</button></>}>
          <div className="stack" style={{ gap: 12 }}>
            <div className="field"><label htmlFor="tn">{t("field.name")}</label><input id="tn" className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} aria-invalid={!!err("name")} />{err("name") && <p className="hint" style={{ color: "var(--danger)" }}>{err("name")}</p>}</div>
            <div className="row" style={{ gap: 12, alignItems: "flex-start" }}>
              <div className="field" style={{ flex: 1 }}><label htmlFor="ts">{t("ac.firstDay")}</label><input id="ts" className="input" type="date" value={form.starts_on} onChange={(e) => setForm({ ...form, starts_on: e.target.value })} aria-invalid={!!err("starts_on")} />{err("starts_on") && <p className="hint" style={{ color: "var(--danger)" }}>{err("starts_on")}</p>}</div>
              <div className="field" style={{ flex: 1 }}><label htmlFor="te">{t("ac.lastDay")}</label><input id="te" className="input" type="date" value={form.ends_on} onChange={(e) => setForm({ ...form, ends_on: e.target.value })} aria-invalid={!!err("ends_on")} />{err("ends_on") && <p className="hint" style={{ color: "var(--danger)" }}>{err("ends_on")}</p>}</div>
            </div>
            {createTerm.error && !createTerm.error.errors.length && <p role="alert" className="alert alert-danger">{createTerm.error.detail ?? createTerm.error.title}</p>}
          </div>
        </Dialog>
      )}
      {dialog && dialog !== "term" && (
        <Dialog title={t("ac.changeTitle", { code: dialog.code })} description={t("ac.changeLead", { title: dialog.title, owner: `${dialog.owner_name} ${dialog.owner_surname}` })} onClose={() => setDialog(null)}
          actions={<><button type="button" className="btn" onClick={() => setDialog(null)}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={!owner || change.pending} onClick={async () => { if (await change.run(dialog.course_id, owner)) setDialog(null); }}>{t("ac.changeOwner")}</button></>}>
          <div className="field"><label htmlFor="own">{t("cs.newOwner")}</label><select id="own" className="select" value={owner} onChange={(e) => setOwner(e.target.value)}><option value="">{t("cs.chooseTeacher")}</option>{candidates.filter((u) => u.id !== dialog.owner_id && u.status === "active").map((u) => <option key={u.id} value={u.id}>{u.name} {u.surname}</option>)}</select></div>
          {change.error && <p role="alert" className="alert alert-danger" style={{ marginTop: 10 }}>{change.error.errors[0]?.message ?? change.error.detail ?? change.error.title}</p>}
        </Dialog>
      )}
    </>
  );
}
