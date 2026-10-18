"use client";

import { useMemo, useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useAdminUserFacts, useAdminUserSummary, useAdminUsers, useCreateUser, useResetPassword, useUpdateUser } from "@/lib/api/hooks/admin";
import type { Role, User, UserFacts } from "@/lib/api/types";
import { dayLabel, dayLong, initials } from "@/lib/format";
import { api } from "@/lib/api/client";
import { invalidate } from "@/lib/query";
import { translate, translatePlural } from "@/lib/i18n/core";
import type { MessageKey } from "@/lib/i18n/core";
import { useT } from "@/lib/i18n";
import s from "./Admin.module.css";

const FILTERS = [["all", "ap.all"], ["student", "ap.students"], ["teacher", "ap.teachers"], ["admin", "ap.admins"], ["disabled", "ap.disabled"]] as const satisfies readonly (readonly [string, MessageKey])[];
type Filter = (typeof FILTERS)[number][0];
const ROLE: Record<Role, MessageKey> = { student: "role.student", teacher: "role.teacher", admin: "ap.admin" };
const PAGE = 8;

/** The facts of a person as rows of the details list (ADR-0053: the server sends data, the screen words it). */
function factRows(f: UserFacts): { k: string; v: string }[] {
  const t = translate;
  const none = t("fact.none");
  const codes = (list: UserFacts["owns"]) => list.map((c) => c.code || c.title).join(", ");
  const rows = [{ k: t("fact.lastSignIn"), v: f.last_login_at ? dayLabel(f.last_login_at) : t("fact.never") }];
  if (f.role === "student") {
    rows.push({ k: t("fact.groups"), v: f.groups.length ? f.groups.map((g) => g.name).join(", ") : none });
    rows.push(f.status === "disabled" ? { k: t("fact.grades"), v: t("fact.gradesKept") } : { k: t("fact.courses"), v: t("fact.studies", { n: f.studies }) });
  } else {
    if (f.owns.length) rows.push({ k: t("fact.owns"), v: codes(f.owns) });
    if (f.teaches.length) rows.push({ k: t("fact.teaches"), v: codes(f.teaches) });
    if (!f.owns.length && !f.teaches.length) rows.push({ k: t("fact.courses"), v: none });
  }
  rows.push({ k: t("fact.added"), v: dayLong(f.created_at) });
  return rows;
}

/** What stops a role change, or what a disabled account means. */
function factNote(f: UserFacts, name: string): string | null {
  if (f.status === "disabled") return translate("note.disabled", { name });
  if (f.role === "teacher" && f.owns.length) return translatePlural("note.owns", f.owns.length, { name, n: f.owns.length, codes: f.owns.map((c) => c.code || c.title).join(", ") });
  return null;
}

function status(u: User) {
  if (u.status === "disabled") return <span className="pill pill-dashed">{translate("ap.disabled")}</span>;
  if (u.must_change_password) return <span className="pill pill-accent">{translate("ap.mustChange")}</span>;
  return <span className="pill pill-ok">{translate("ap.active")}</span>;
}

function Password({ title, name, password, onClose }: { title: string; name: string; password: string; onClose: () => void }) {
  const t = useT();
  return (
    <Dialog title={title} description={t("ap.giveTo", { name })} onClose={onClose} actions={<button type="button" className="btn btn-primary" onClick={onClose}>{t("common.done")}</button>}>
      <code className={s.tpw} aria-label={t("change.current")}>{password}</code>
      <button type="button" className="btn" onClick={() => void navigator.clipboard?.writeText(password)}><Icon name="copy" size={15} />{t("be.copy")}</button>
    </Dialog>
  );
}

function AddPerson({ onClose, onCreated }: { onClose: () => void; onCreated: (u: User, pw: string) => void }) {
  const t = useT();
  const create = useCreateUser();
  const [f, setF] = useState({ name: "", surname: "", email: "", role: "student" as Role });
  const err = (k: string) => create.error?.errors.find((e) => e.field === k)?.message;
  async function submit() {
    const r = await create.run(f);
    if (r) onCreated(r.user, r.temporary_password);
  }
  return (
    <Dialog title={t("ap.addPerson")} description={t("ap.addLead")} onClose={onClose}
      actions={<><button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={create.pending} onClick={submit}>{t("ap.addPersonBtn")}</button></>}>
      <div className="stack" style={{ gap: 12 }}>
        <div className="row" style={{ gap: 12, alignItems: "flex-start" }}>
          <div className="field" style={{ flex: 1 }}><label htmlFor="n">{t("field.firstName")}</label><input id="n" className="input" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} aria-invalid={!!err("name")} />{err("name") && <p className="hint" style={{ color: "var(--danger)" }}>{err("name")}</p>}</div>
          <div className="field" style={{ flex: 1 }}><label htmlFor="sn">{t("field.lastName")}</label><input id="sn" className="input" value={f.surname} onChange={(e) => setF({ ...f, surname: e.target.value })} aria-invalid={!!err("surname")} />{err("surname") && <p className="hint" style={{ color: "var(--danger)" }}>{err("surname")}</p>}</div>
        </div>
        <div className="field"><label htmlFor="em">{t("field.email")}</label><input id="em" className="input" type="email" value={f.email} onChange={(e) => setF({ ...f, email: e.target.value })} aria-invalid={!!err("email")} />{err("email") && <p className="hint" style={{ color: "var(--danger)" }}>{err("email")}</p>}</div>
        <div className="field"><label htmlFor="rl">{t("field.role")}</label><select id="rl" className="select" value={f.role} onChange={(e) => setF({ ...f, role: e.target.value as Role })}><option value="student">{t("role.student")}</option><option value="teacher">{t("role.teacher")}</option><option value="admin">{t("ap.admin")}</option></select></div>
        {create.error && !create.error.errors.length && <p role="alert" className="alert alert-danger">{create.error.detail ?? create.error.title}</p>}
      </div>
    </Dialog>
  );
}

/** Import people from CSV text: email, name, surname, role per line. Each line is one create call. */
function ImportCsv({ onClose }: { onClose: () => void }) {
  const t = useT();
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
    <Dialog title={t("ap.csv")} description={t("ap.csvLead")} onClose={onClose}
      actions={result ? <button type="button" className="btn btn-primary" onClick={onClose}>{t("common.done")}</button> : <><button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={busy || !text.trim()} onClick={run}>{t("ap.import")}</button></>}>
      {result ? (
        <div className="stack" style={{ gap: 10 }}>
          <p role="status">{t.plural("ap.added", result.ok.length)}</p>
          <ul style={{ fontSize: 14 }}>{result.ok.map((o) => <li key={o.name}>{o.name}: <code>{o.password}</code></li>)}</ul>
          {result.failed.length > 0 && <div role="alert" className="alert alert-danger"><ul>{result.failed.map((f) => <li key={f}>{f}</li>)}</ul></div>}
        </div>
      ) : (
        <>
          <div className="field"><label htmlFor="csv">CSV</label><textarea id="csv" className="textarea" style={{ minHeight: 150, fontFamily: "var(--font-mono), monospace", fontSize: 13 }} placeholder="new.student@northfield.edu, Nina, Hrytsenko, student" value={text} onChange={(e) => setText(e.target.value)} /></div>
          <label className="btn" style={{ marginTop: 10 }}>{t("ap.chooseFile")}<input type="file" accept=".csv,text/csv" className="sr-only" onChange={async (e) => { const file = e.target.files?.[0]; if (file) setText(await file.text()); }} /></label>
        </>
      )}
    </Dialog>
  );
}

export function AdminPeople() {
  const t = useT();
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
          <h1>{t("ap.people")}</h1>
          <p className="muted" style={{ marginTop: 4 }}>{count ? `${t.plural("ap.accounts", count.all)} ` : ""}{t("ap.addEveryone")}</p>
        </div>
        <button type="button" className="btn" onClick={() => setDialog("csv")}>{t("ap.csv")}</button>
        <button type="button" className="btn btn-primary" onClick={() => setDialog("add")}><Icon name="plus" size={16} strokeWidth={2.2} />{t("ap.addPerson")}</button>
      </div>
      <div role="group" aria-label={t("ap.filterRole")} className={s.chips}>
        {FILTERS.map(([id, label]) => (
          <button key={id} type="button" className={s.chip} aria-pressed={filter === id} onClick={() => { setFilter(id); setShown(PAGE); setSel(null); }}>
            {t(label)} <span>{count ? (id === "all" ? count.all : count[id]) : ""}</span>
          </button>
        ))}
      </div>
      {!users.data ? <p className="loading" role="status">{t("common.loading")}</p> : (
        <div className={s.split}>
          <div className={`card ${s.tableCard}`}>
            <div className="table-wrap">
              <table className="table">
                <thead><tr><th scope="col">{t("field.name")}</th><th scope="col">{t("field.role")}</th><th scope="col">{t("ap.status")}</th></tr></thead>
                <tbody>
                  {list.slice(0, shown).map((u) => (
                    <tr key={u.id} className={s.row} aria-selected={chosen?.id === u.id} onClick={() => setSel(u.id)}>
                      <th scope="row">
                        <button type="button" className={s.person} style={{ border: 0, background: "transparent", padding: 0 }} onClick={() => setSel(u.id)}>
                          <span aria-hidden="true" className={s.avatar}>{initials(u.name, u.surname)}</span>
                          <span><span style={{ display: "block", fontWeight: 500, fontSize: 14, color: "var(--text)" }}>{u.name} {u.surname}</span><span className="muted" style={{ display: "block", fontSize: 13, fontWeight: 400 }}>{u.email}</span></span>
                        </button>
                      </th>
                      <td>{t(ROLE[u.role])}</td>
                      <td>{status(u)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {list.length === 0 && <p className="empty">{t("people.nobodyMatches")}</p>}
            </div>
            {shown < list.length && <div style={{ padding: 12, textAlign: "center", borderTop: "1px solid var(--border)" }}><button type="button" className="btn btn-quiet" onClick={() => setShown((n) => n + PAGE)}>{t("ap.showMore")}</button></div>}
          </div>
          {chosen && (
            <aside aria-labelledby="pd-h" className={`card ${s.detail}`}>
              <div className="row" style={{ gap: 12 }}>
                <span aria-hidden="true" className={s.avatar} data-big="true">{initials(chosen.name, chosen.surname)}</span>
                <div style={{ minWidth: 0 }}><h2 id="pd-h" style={{ fontSize: 18, fontWeight: 600 }}>{chosen.name} {chosen.surname}</h2><p className="muted ellipsis" style={{ fontSize: 14 }}>{chosen.email}</p></div>
              </div>
              <div className="field">
                <label htmlFor="role">{t("field.role")}</label>
                <select id="role" className="select" value={chosen.role} onChange={(e) => void update.run(chosen.id, { role: e.target.value as Role })}>
                  <option value="student">{t("role.student")}</option><option value="teacher">{t("role.teacher")}</option><option value="admin">{t("ap.admin")}</option>
                </select>
              </div>
              {update.error && <p role="alert" className="alert alert-danger">{update.error.detail ?? update.error.title}</p>}
              {facts.data && factNote(facts.data, chosen.name) && <p className="muted" style={{ fontSize: 13 }}>{factNote(facts.data, chosen.name)}</p>}
              <dl className={s.dl}>
                {(facts.data ? factRows(facts.data) : []).map((x) => <div key={x.k} style={{ display: "contents" }}><dt>{x.k}</dt><dd>{x.v}</dd></div>)}
              </dl>
              <div className="stack" style={{ gap: 8 }}>
                <button type="button" className="btn btn-block" disabled={reset.pending} onClick={async () => { const r = await reset.run(chosen.id); if (r) setPw({ title: t("ap.tempSet"), name: chosen.name, password: r.temporary_password }); }}>{t("ap.setTemp")}</button>
                <p className="muted" style={{ fontSize: 13 }}>{t("ap.tempNote")}</p>
                <button type="button" className={`btn btn-block ${chosen.status === "active" ? "btn-danger" : ""}`} onClick={() => void update.run(chosen.id, { status: chosen.status === "active" ? "disabled" : "active" })}>{chosen.status === "disabled" ? t("ap.enable") : t("ap.disable")}</button>
              </div>
            </aside>
          )}
        </div>
      )}
      {dialog === "add" && <AddPerson onClose={() => setDialog(null)} onCreated={(u, p) => { setDialog(null); setSel(u.id); setPw({ title: t("ap.personAdded"), name: u.name, password: p }); }} />}
      {dialog === "csv" && <ImportCsv onClose={() => setDialog(null)} />}
      {pw && <Password {...pw} onClose={() => setPw(null)} />}
    </>
  );
}
