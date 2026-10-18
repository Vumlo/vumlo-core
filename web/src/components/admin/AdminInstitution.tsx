"use client";

import { useState } from "react";
import { useAdminInstitution, useSaveInstitution } from "@/lib/api/hooks/admin";
import { dayLabel } from "@/lib/format";
import { setTimeZone } from "@/lib/format";
import { useT } from "@/lib/i18n";
import s from "./Admin.module.css";

const ZONES = ["Europe/Kyiv", "Europe/Warsaw", "UTC"];

export function AdminInstitution() {
  const t = useT();
  const inst = useAdminInstitution();
  const save = useSaveInstitution();
  const [draft, setDraft] = useState<{ name?: string; short_name?: string; accent?: string; language?: "en" | "uk"; time_zone?: string }>({});
  const [saved, setSaved] = useState(false);
  const i = inst.data;
  if (!i) return <p className="loading" role="status">{t("common.loading")}</p>;
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
      <div className={s.head}><div><h1>{t("ai.title")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("ai.lead")}</p></div></div>
      <div className="stack" style={{ gap: 20 }}>
        <section aria-labelledby="in-h" className={`card ${s.section}`}>
          <h2 id="in-h" style={{ fontSize: 18, fontWeight: 600, marginBottom: 14 }}>{t("ai.nameLook")}</h2>
          <div className="row" style={{ gap: 28, flexWrap: "wrap", alignItems: "flex-start" }}>
            <div className="stack" style={{ gap: 14, flex: "1 1 280px" }}>
              <div className="field"><label htmlFor="in-name">{t("ai.fullName")}</label><input id="in-name" className="input" value={v.name} onChange={(e) => set({ name: e.target.value })} aria-invalid={!!err("name")} />{err("name") && <p className="hint" style={{ color: "var(--danger)" }}>{err("name")}</p>}</div>
              <div className="field"><label htmlFor="in-short">{t("ai.shortName")}</label><input id="in-short" className="input" value={v.short_name} onChange={(e) => set({ short_name: e.target.value })} /><p className="hint">{t("ai.shortHint")}</p></div>
              <div>
                <p style={{ fontSize: 14, fontWeight: 500, marginBottom: 8 }}>{t("ai.accent")}</p>
                <div role="radiogroup" aria-label={t("ai.accent")} className={s.swatches}>
                  {i.accents.map((a) => (
                    <button key={a.name} type="button" role="radio" aria-checked={v.accent === a.name} aria-label={a.name} className={s.swatch} onClick={() => set({ accent: a.name })}><span className={s.dot} style={{ background: swatch(a.name) }} /></button>
                  ))}
                </div>
                <p className="hint" style={{ marginTop: 6 }}>{t("ai.accentHint")}</p>
              </div>
            </div>
            <div style={{ flex: "1 1 240px" }}>
              <p style={{ fontSize: 14, fontWeight: 500, marginBottom: 8 }}>{t("ai.logo")}</p>
              <div className="row" style={{ gap: 14, marginBottom: 16 }}>
                <span aria-hidden="true" className={s.logo} style={{ width: 56, height: 56, borderRadius: 12, fontSize: 24, background: swatch(v.accent) }}>{v.name.charAt(0)}</span>
                <div><button type="button" className="btn" disabled title={t("ai.uploadLater")}>{t("ai.upload")}</button><p className="muted" style={{ fontSize: 13, marginTop: 6 }}>{t("ai.logoHint")}</p></div>
              </div>
              <p style={{ fontSize: 14, fontWeight: 500, marginBottom: 8 }}>{t("ai.preview")}</p>
              <div className={s.preview}><span aria-hidden="true" className={s.logo} style={{ background: swatch(v.accent) }}>{v.name.charAt(0)}</span><span style={{ fontWeight: 600 }}>{v.name}</span><span className="muted" style={{ marginLeft: "auto", fontSize: 14 }}>{t("nav.home")}</span></div>
            </div>
          </div>
        </section>
        <section aria-labelledby="lg-h" className={`card ${s.section}`}>
          <h2 id="lg-h" style={{ fontSize: 18, fontWeight: 600, marginBottom: 14 }}>{t("ai.langTime")}</h2>
          <div className="row" style={{ gap: 20, flexWrap: "wrap", alignItems: "flex-start" }}>
            <div className="field" style={{ flex: "1 1 240px" }}><label htmlFor="in-lang">{t("ai.defaultLang")}</label><select id="in-lang" className="select" value={v.language} onChange={(e) => set({ language: e.target.value as "en" | "uk" })}><option value="en">English</option><option value="uk">Українська</option></select><p className="hint">{t("ai.langHint")}</p></div>
            <div className="field" style={{ flex: "1 1 240px" }}><label htmlFor="in-tz">{t("ai.timeZone")}</label><select id="in-tz" className="select" value={v.time_zone} onChange={(e) => set({ time_zone: e.target.value })}>{ZONES.map((z) => <option key={z}>{z}</option>)}</select><p className="hint">{t("ai.zoneHint")}</p></div>
          </div>
        </section>
        <section aria-labelledby="sv-h" className={`card ${s.section}`}>
          <h2 id="sv-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("ai.server")}</h2>
          <p className="muted" style={{ margin: "4px 0 14px", fontSize: 14 }}>{t("ai.serverLead")}</p>
          <dl className={s.dl} style={{ gridTemplateColumns: "180px 1fr", fontSize: 15 }}>
            <dt>{t("ai.scale")}</dt><dd>{i.server.grading_scale}</dd>
            <dt>{t("ai.wrongPw")}</dt><dd>{i.server.failed_logins}</dd>
            <dt>{t("profile.section.sessions")}</dt><dd>{i.server.session_lifetime}</dd>
            <dt>{t("ai.largest")}</dt><dd>{i.server.largest_document}</dd>
          </dl>
        </section>
        <div className="row" style={{ gap: 14 }}>
          <button type="button" className="btn btn-primary" disabled={Object.keys(draft).length === 0 || save.pending} onClick={submit}>{t("cs.save")}</button>
          <span role="status" className="muted" style={{ fontSize: 14 }}>{saved ? t("cs.saved") : i.changed_at ? t("ai.lastChanged", { day: dayLabel(i.changed_at), who: i.changed_by ?? "" }) : ""}</span>
        </div>
      </div>
    </>
  );
}
