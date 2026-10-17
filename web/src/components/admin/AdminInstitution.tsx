"use client";

import { useState } from "react";
import { useAdminInstitution, useSaveInstitution } from "@/lib/api/hooks/admin";
import { dayLabel } from "@/lib/format";
import { setTimeZone } from "@/lib/format";
import s from "./Admin.module.css";

const ZONES = ["Europe/Kyiv", "Europe/Warsaw", "UTC"];

export function AdminInstitution() {
  const inst = useAdminInstitution();
  const save = useSaveInstitution();
  const [draft, setDraft] = useState<{ name?: string; short_name?: string; accent?: string; language?: "en" | "uk"; time_zone?: string }>({});
  const [saved, setSaved] = useState(false);
  const i = inst.data;
  if (!i) return <p className="loading" role="status">Loading…</p>;
  const v = { name: draft.name ?? i.name, short_name: draft.short_name ?? i.short_name, accent: draft.accent ?? i.accent, language: draft.language ?? i.language, time_zone: draft.time_zone ?? i.time_zone };
  const set = (patch: typeof draft) => { setDraft((d) => ({ ...d, ...patch })); setSaved(false); };
  const err = (f: string) => save.error?.errors.find((e) => e.field === f)?.message;
  const swatch = (name: string) => `var(--swatch-${name.toLowerCase()})`;

  async function submit() {
    const r = await save.run(draft);
    if (r) { setDraft({}); setSaved(true); setTimeZone(r.time_zone); }
  }

  return (
    <>
      <div className={s.head}><div><h1>Institution</h1><p className="muted" style={{ marginTop: 4 }}>What everyone sees across the site.</p></div></div>
      <div className="stack" style={{ gap: 20 }}>
        <section aria-labelledby="in-h" className={`card ${s.section}`}>
          <h2 id="in-h" style={{ fontSize: 18, fontWeight: 600, marginBottom: 14 }}>Name and look</h2>
          <div className="row" style={{ gap: 28, flexWrap: "wrap", alignItems: "flex-start" }}>
            <div className="stack" style={{ gap: 14, flex: "1 1 280px" }}>
              <div className="field"><label htmlFor="in-name">Full name</label><input id="in-name" className="input" value={v.name} onChange={(e) => set({ name: e.target.value })} aria-invalid={!!err("name")} />{err("name") && <p className="hint" style={{ color: "var(--danger)" }}>{err("name")}</p>}</div>
              <div className="field"><label htmlFor="in-short">Short name</label><input id="in-short" className="input" value={v.short_name} onChange={(e) => set({ short_name: e.target.value })} /><p className="hint">Used on phones and in emails</p></div>
              <div>
                <p style={{ fontSize: 14, fontWeight: 500, marginBottom: 8 }}>Accent colour</p>
                <div role="radiogroup" aria-label="Accent colour" className={s.swatches}>
                  {i.accents.map((a) => (
                    <button key={a.name} type="button" role="radio" aria-checked={v.accent === a.name} aria-label={a.name} className={s.swatch} onClick={() => set({ accent: a.name })}><span className={s.dot} style={{ background: swatch(a.name) }} /></button>
                  ))}
                </div>
                <p className="hint" style={{ marginTop: 6 }}>Shown in the preview. The site keeps its tested accent until each colour has a dark-theme match.</p>
              </div>
            </div>
            <div style={{ flex: "1 1 240px" }}>
              <p style={{ fontSize: 14, fontWeight: 500, marginBottom: 8 }}>Logo</p>
              <div className="row" style={{ gap: 14, marginBottom: 16 }}>
                <span aria-hidden="true" className={s.logo} style={{ width: 56, height: 56, borderRadius: 12, fontSize: 24, background: swatch(v.accent) }}>{v.name.charAt(0)}</span>
                <div><button type="button" className="btn" disabled title="Uploads come later, with file uploads">Upload</button><p className="muted" style={{ fontSize: 13, marginTop: 6 }}>Square SVG or PNG. Without one, we show the first letter.</p></div>
              </div>
              <p style={{ fontSize: 14, fontWeight: 500, marginBottom: 8 }}>Preview</p>
              <div className={s.preview}><span aria-hidden="true" className={s.logo} style={{ background: swatch(v.accent) }}>{v.name.charAt(0)}</span><span style={{ fontWeight: 600 }}>{v.name}</span><span className="muted" style={{ marginLeft: "auto", fontSize: 14 }}>Home</span></div>
            </div>
          </div>
        </section>
        <section aria-labelledby="lg-h" className={`card ${s.section}`}>
          <h2 id="lg-h" style={{ fontSize: 18, fontWeight: 600, marginBottom: 14 }}>Language and time</h2>
          <div className="row" style={{ gap: 20, flexWrap: "wrap", alignItems: "flex-start" }}>
            <div className="field" style={{ flex: "1 1 240px" }}><label htmlFor="in-lang">Default language</label><select id="in-lang" className="select" value={v.language} onChange={(e) => set({ language: e.target.value as "en" | "uk" })}><option value="en">English</option><option value="uk">Українська</option></select><p className="hint">People can pick their own in their profile</p></div>
            <div className="field" style={{ flex: "1 1 240px" }}><label htmlFor="in-tz">Time zone</label><select id="in-tz" className="select" value={v.time_zone} onChange={(e) => set({ time_zone: e.target.value })}>{ZONES.map((z) => <option key={z}>{z}</option>)}</select><p className="hint">Deadlines are shown in this zone</p></div>
          </div>
        </section>
        <section aria-labelledby="sv-h" className={`card ${s.section}`}>
          <h2 id="sv-h" style={{ fontSize: 18, fontWeight: 600 }}>Set on the server</h2>
          <p className="muted" style={{ margin: "4px 0 14px", fontSize: 14 }}>These come from the server&apos;s config file. Ask whoever runs the server to change them.</p>
          <dl className={s.dl} style={{ gridTemplateColumns: "180px 1fr", fontSize: 15 }}>
            <dt>Grade scale</dt><dd>{i.server.grading_scale}</dd>
            <dt>Wrong passwords</dt><dd>{i.server.failed_logins}</dd>
            <dt>Sessions</dt><dd>{i.server.session_lifetime}</dd>
            <dt>Largest document</dt><dd>{i.server.largest_document}</dd>
          </dl>
        </section>
        <div className="row" style={{ gap: 14 }}>
          <button type="button" className="btn btn-primary" disabled={Object.keys(draft).length === 0 || save.pending} onClick={submit}>Save changes</button>
          <span role="status" className="muted" style={{ fontSize: 14 }}>{saved ? "Saved." : i.changed_at ? `Last changed ${dayLabel(i.changed_at)} by ${i.changed_by}` : ""}</span>
        </div>
      </div>
    </>
  );
}
