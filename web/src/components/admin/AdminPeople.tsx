"use client";

import { useMemo, useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useAdminUserFacts, useAdminUserSummary, useAdminUsers, useCreateUser, useResetPassword, useUpdateUser } from "@/lib/api/hooks/admin";
import type { Role, User } from "@/lib/api/types";
import { initials } from "@/lib/format";
import { api } from "@/lib/api/client";
import { invalidate } from "@/lib/query";
import s from "./Admin.module.css";

const FILTERS = [["all", "All"], ["student", "Students"], ["teacher", "Teachers"], ["admin", "Admins"], ["disabled", "Disabled"]] as const;
type Filter = (typeof FILTERS)[number][0];
const ROLE: Record<Role, string> = { student: "Student", teacher: "Teacher", admin: "Admin" };
const PAGE = 8;

function status(u: User) {
  if (u.status === "disabled") return <span className="pill pill-dashed">Disabled</span>;
  if (u.must_change_password) return <span className="pill pill-accent">Must change password</span>;
  return <span className="pill pill-ok">Active</span>;
}

function Password({ title, name, password, onClose }: { title: string; name: string; password: string; onClose: () => void }) {
  return (
    <Dialog title={title} description={`Give this to ${name}. It is shown once, and they must change it the next time they sign in.`} onClose={onClose} actions={<button type="button" className="btn btn-primary" onClick={onClose}>Done</button>}>
      <code className={s.tpw} aria-label="Temporary password">{password}</code>
      <button type="button" className="btn" onClick={() => void navigator.clipboard?.writeText(password)}><Icon name="copy" size={15} />Copy</button>
    </Dialog>
  );
}

function AddPerson({ onClose, onCreated }: { onClose: () => void; onCreated: (u: User, pw: string) => void }) {
  const create = useCreateUser();
  const [f, setF] = useState({ name: "", surname: "", email: "", role: "student" as Role });
  const err = (k: string) => create.error?.errors.find((e) => e.field === k)?.message;
  async function submit() {
    const r = await create.run(f);
    if (r) onCreated(r.user, r.temporary_password);
  }
  return (
    <Dialog title="Add a person" description="They get a temporary password and choose their own at first sign-in." onClose={onClose}
      actions={<><button type="button" className="btn" onClick={onClose}>Cancel</button><button type="button" className="btn btn-primary" disabled={create.pending} onClick={submit}>Add person</button></>}>
      <div className="stack" style={{ gap: 12 }}>
        <div className="row" style={{ gap: 12, alignItems: "flex-start" }}>
          <div className="field" style={{ flex: 1 }}><label htmlFor="n">First name</label><input id="n" className="input" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} aria-invalid={!!err("name")} />{err("name") && <p className="hint" style={{ color: "var(--danger)" }}>{err("name")}</p>}</div>
          <div className="field" style={{ flex: 1 }}><label htmlFor="sn">Surname</label><input id="sn" className="input" value={f.surname} onChange={(e) => setF({ ...f, surname: e.target.value })} aria-invalid={!!err("surname")} />{err("surname") && <p className="hint" style={{ color: "var(--danger)" }}>{err("surname")}</p>}</div>
        </div>
        <div className="field"><label htmlFor="em">Email</label><input id="em" className="input" type="email" value={f.email} onChange={(e) => setF({ ...f, email: e.target.value })} aria-invalid={!!err("email")} />{err("email") && <p className="hint" style={{ color: "var(--danger)" }}>{err("email")}</p>}</div>
        <div className="field"><label htmlFor="rl">Role</label><select id="rl" className="select" value={f.role} onChange={(e) => setF({ ...f, role: e.target.value as Role })}><option value="student">Student</option><option value="teacher">Teacher</option><option value="admin">Admin</option></select></div>
        {create.error && !create.error.errors.length && <p role="alert" className="alert alert-danger">{create.error.detail ?? create.error.title}</p>}
      </div>
    </Dialog>
  );
}

/** Import people from CSV text: email, name, surname, role per line. Each line is one create call. */
function ImportCsv({ onClose }: { onClose: () => void }) {
  const [text, setText] = useState("");
  const [result, setResult] = useState<{ ok: { name: string; password: string }[]; failed: string[] } | null>(null);
  const [busy, setBusy] = useState(false);
  async function run() {
    setBusy(true);
    const ok: { name: string; password: string }[] = [];
    const failed: string[] = [];
    for (const line of text.split("\n").map((l) => l.trim()).filter(Boolean)) {
      const [email = "", name = "", surname = "", role = "student"] = line.split(",").map((x) => x.trim());
      try {
        const r = await api.post<{ user: User; temporary_password: string }>("/admin/users", { email, name, surname, role });
        ok.push({ name: `${r.user.name} ${r.user.surname}`, password: r.temporary_password });
      } catch (e) {
        failed.push(`${email || line}: ${(e as Error).message}`);
      }
    }
    invalidate("/admin/");
    setResult({ ok, failed });
    setBusy(false);
  }
  return (
    <Dialog title="Import from CSV" description="One person per line: email, first name, surname, role (student, teacher or admin)." onClose={onClose}
      actions={result ? <button type="button" className="btn btn-primary" onClick={onClose}>Done</button> : <><button type="button" className="btn" onClick={onClose}>Cancel</button><button type="button" className="btn btn-primary" disabled={busy || !text.trim()} onClick={run}>Import</button></>}>
      {result ? (
        <div className="stack" style={{ gap: 10 }}>
          <p role="status">Added {result.ok.length} {result.ok.length === 1 ? "person" : "people"}. Their temporary passwords are shown once:</p>
          <ul style={{ fontSize: 14 }}>{result.ok.map((o) => <li key={o.name}>{o.name}: <code>{o.password}</code></li>)}</ul>
          {result.failed.length > 0 && <div role="alert" className="alert alert-danger"><ul>{result.failed.map((f) => <li key={f}>{f}</li>)}</ul></div>}
        </div>
      ) : (
        <>
          <div className="field"><label htmlFor="csv">CSV</label><textarea id="csv" className="textarea" style={{ minHeight: 150, fontFamily: "var(--font-mono), monospace", fontSize: 13 }} placeholder="new.student@northfield.edu, Nina, Hrytsenko, student" value={text} onChange={(e) => setText(e.target.value)} /></div>
          <label className="btn" style={{ marginTop: 10 }}>Choose a file<input type="file" accept=".csv,text/csv" className="sr-only" onChange={async (e) => { const file = e.target.files?.[0]; if (file) setText(await file.text()); }} /></label>
        </>
      )}
    </Dialog>
  );
}

export function AdminPeople() {
  const [filter, setFilter] = useState<Filter>("all");
  const users = useAdminUsers(filter === "all" ? {} : filter === "disabled" ? { status: "disabled" } : { role: filter });
  const summary = useAdminUserSummary();
  const [sel, setSel] = useState<string | null>(null);
  const [shown, setShown] = useState(PAGE);
  const [dialog, setDialog] = useState<null | "add" | "csv">(null);
  const [pw, setPw] = useState<null | { title: string; name: string; password: string }>(null);
  const update = useUpdateUser();
  const reset = useResetPassword();

  const list = users.data ?? [];
  const chosen = list.find((u) => u.id === sel) ?? list[0];
  const facts = useAdminUserFacts(chosen?.id ?? null);
  const count = useMemo(() => summary.data, [summary.data]);

  return (
    <>
      <div className={s.head}>
        <div>
          <h1>People</h1>
          <p className="muted" style={{ marginTop: 4 }}>{count ? `${count.all} accounts. ` : ""}Nobody can sign up on their own, you add everyone here.</p>
        </div>
        <button type="button" className="btn" onClick={() => setDialog("csv")}>Import from CSV</button>
        <button type="button" className="btn btn-primary" onClick={() => setDialog("add")}><Icon name="plus" size={16} strokeWidth={2.2} />Add a person</button>
      </div>
      <div role="group" aria-label="Filter by role" className={s.chips}>
        {FILTERS.map(([id, label]) => (
          <button key={id} type="button" className={s.chip} aria-pressed={filter === id} onClick={() => { setFilter(id); setShown(PAGE); setSel(null); }}>
            {label} <span>{count ? (id === "all" ? count.all : count[id]) : ""}</span>
          </button>
        ))}
      </div>
      {!users.data ? <p className="loading" role="status">Loading…</p> : (
        <div className={s.split}>
          <div className={`card ${s.tableCard}`}>
            <div className="table-wrap">
              <table className="table">
                <thead><tr><th scope="col">Name</th><th scope="col">Role</th><th scope="col">Status</th></tr></thead>
                <tbody>
                  {list.slice(0, shown).map((u) => (
                    <tr key={u.id} className={s.row} aria-selected={chosen?.id === u.id} onClick={() => setSel(u.id)}>
                      <th scope="row">
                        <button type="button" className={s.person} style={{ border: 0, background: "transparent", padding: 0 }} onClick={() => setSel(u.id)}>
                          <span aria-hidden="true" className={s.avatar}>{initials(u.name, u.surname)}</span>
                          <span><span style={{ display: "block", fontWeight: 500, fontSize: 14, color: "var(--text)" }}>{u.name} {u.surname}</span><span className="muted" style={{ display: "block", fontSize: 13, fontWeight: 400 }}>{u.email}</span></span>
                        </button>
                      </th>
                      <td>{ROLE[u.role]}</td>
                      <td>{status(u)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {list.length === 0 && <p className="empty">Nobody matches.</p>}
            </div>
            {shown < list.length && <div style={{ padding: 12, textAlign: "center", borderTop: "1px solid var(--border)" }}><button type="button" className="btn btn-quiet" onClick={() => setShown((n) => n + PAGE)}>Show more</button></div>}
          </div>
          {chosen && (
            <aside aria-labelledby="pd-h" className={`card ${s.detail}`}>
              <div className="row" style={{ gap: 12 }}>
                <span aria-hidden="true" className={s.avatar} data-big="true">{initials(chosen.name, chosen.surname)}</span>
                <div style={{ minWidth: 0 }}><h2 id="pd-h" style={{ fontSize: 18, fontWeight: 600 }}>{chosen.name} {chosen.surname}</h2><p className="muted ellipsis" style={{ fontSize: 14 }}>{chosen.email}</p></div>
              </div>
              <div className="field">
                <label htmlFor="role">Role</label>
                <select id="role" className="select" value={chosen.role} onChange={(e) => void update.run(chosen.id, { role: e.target.value as Role })}>
                  <option value="student">Student</option><option value="teacher">Teacher</option><option value="admin">Admin</option>
                </select>
              </div>
              {update.error && <p role="alert" className="alert alert-danger">{update.error.detail ?? update.error.title}</p>}
              {facts.data?.role_note && <p className="muted" style={{ fontSize: 13 }}>{facts.data.role_note}</p>}
              <dl className={s.dl}>
                {(facts.data?.facts ?? []).map((x) => <div key={x.k} style={{ display: "contents" }}><dt>{x.k}</dt><dd>{x.v}</dd></div>)}
              </dl>
              <div className="stack" style={{ gap: 8 }}>
                <button type="button" className="btn btn-block" disabled={reset.pending} onClick={async () => { const r = await reset.run(chosen.id); if (r) setPw({ title: "Temporary password set", name: chosen.name, password: r.temporary_password }); }}>Set a temporary password</button>
                <p className="muted" style={{ fontSize: 13 }}>They have to change it the next time they sign in. Their other sessions end.</p>
                <button type="button" className={`btn btn-block ${chosen.status === "active" ? "btn-danger" : ""}`} onClick={() => void update.run(chosen.id, { status: chosen.status === "active" ? "disabled" : "active" })}>{chosen.status === "disabled" ? "Enable the account" : "Disable the account"}</button>
              </div>
            </aside>
          )}
        </div>
      )}
      {dialog === "add" && <AddPerson onClose={() => setDialog(null)} onCreated={(u, p) => { setDialog(null); setSel(u.id); setPw({ title: "Person added", name: u.name, password: p }); }} />}
      {dialog === "csv" && <ImportCsv onClose={() => setDialog(null)} />}
      {pw && <Password {...pw} onClose={() => setPw(null)} />}
    </>
  );
}
