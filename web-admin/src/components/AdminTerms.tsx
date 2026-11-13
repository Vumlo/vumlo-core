
import { useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { useAdminTermStats, useCreateTerm } from "@/lib/api/hooks/admin";
import { useTerms } from "@/lib/api/hooks/courses";
import { monthShort, plural } from "@/lib/format";
import { translate } from "@/lib/i18n/core";
import { useT } from "@/lib/i18n";
import s from "./Admin.module.css";

const range = (a: string, b: string) => {
  const f = (d: string, year: boolean) => { const [y = "", m = "1", day = "1"] = d.split("-"); return `${Number(day)} ${monthShort(Number(m))}${year ? ` ${y}` : ""}`; };
  return translate("ac.range", { from: f(a, a.slice(0, 4) !== b.slice(0, 4)), to: f(b, true) });
};

/** The terms of the institution. The courses themselves are listed on the courses page (the "All courses" tab). */
export function AdminTerms() {
  const t = useT();
  const terms = useTerms();
  const stats = useAdminTermStats();
  const createTerm = useCreateTerm();
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState({ name: "", starts_on: "", ends_on: "" });

  const err = (f: string) => createTerm.error?.errors.find((e) => e.field === f)?.message;

  return (
    <>
      <div className={s.head}><div><h1>{t("ac.title")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("ac.lead")}</p></div></div>
      <section aria-labelledby="tm-h" className={`card ${s.section}`} style={{ marginBottom: 24 }}>
        <div className="row" style={{ justifyContent: "space-between", marginBottom: 12 }}>
          <h2 id="tm-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("ac.terms")}</h2>
          <button type="button" className="btn" onClick={() => { setForm({ name: "", starts_on: "", ends_on: "" }); createTerm.reset(); setOpen(true); }}><Icon name="plus" size={15} strokeWidth={2.2} />{t("ac.addTerm")}</button>
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
      {open && (
        <Dialog title={t("ac.addTerm")} onClose={() => setOpen(false)} actions={<><button type="button" className="btn" onClick={() => setOpen(false)}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={createTerm.pending} onClick={async () => { if (await createTerm.run(form)) setOpen(false); }}>{t("ac.addTermBtn")}</button></>}>
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
    </>
  );
}
