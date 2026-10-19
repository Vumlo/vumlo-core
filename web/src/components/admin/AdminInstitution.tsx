"use client";

import { useAdminInstitution } from "@/lib/api/hooks/admin";
import { useT } from "@/lib/i18n";
import { AdminAppDownloads } from "./AdminAppDownloads";
import s from "./Admin.module.css";

export function AdminInstitution() {
  const t = useT();
  const inst = useAdminInstitution();
  const i = inst.data;
  if (!i) return <p className="loading" role="status">{t("common.loading")}</p>;

  return (
    <>
      <div className={s.head}><div><h1>{t("ai.title")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("ai.lead")}</p></div></div>
      <div className="stack" style={{ gap: 20 }}>
        <section aria-labelledby="in-h" className={`card ${s.section}`}>
          <h2 id="in-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("ai.nameLook")}</h2>
          <p className="muted" style={{ margin: "4px 0 14px", fontSize: 14 }}>{t("ai.serverLead")}</p>
          <dl className={s.dl} style={{ gridTemplateColumns: "180px 1fr", fontSize: 15 }}>
            <dt>{t("ai.fullName")}</dt><dd>{i.name}</dd>
            <dt>{t("ai.shortName")}</dt><dd>{i.short_name}</dd>
          </dl>
          <p style={{ fontSize: 14, fontWeight: 500, margin: "16px 0 8px" }}>{t("ai.preview")}</p>
          <div className={s.preview}><span aria-hidden="true" className={s.logo} style={{ background: "var(--accent)" }}>{i.name.charAt(0)}</span><span style={{ fontWeight: 600 }}>{i.name}</span><span className="muted" style={{ marginLeft: "auto", fontSize: 14 }}>{t("nav.home")}</span></div>
        </section>
        <AdminAppDownloads current={i.app_downloads} />
        <section aria-labelledby="lg-h" className={`card ${s.section}`}>
          <h2 id="lg-h" style={{ fontSize: 18, fontWeight: 600, marginBottom: 14 }}>{t("ai.langTime")}</h2>
          <dl className={s.dl} style={{ gridTemplateColumns: "180px 1fr", fontSize: 15 }}>
            <dt>{t("ai.defaultLang")}</dt><dd>{i.language === "uk" ? "Українська" : "English"}</dd>
            <dt>{t("ai.timeZone")}</dt><dd>{i.time_zone}</dd>
          </dl>
        </section>
        <section aria-labelledby="sv-h" className={`card ${s.section}`}>
          <h2 id="sv-h" style={{ fontSize: 18, fontWeight: 600, marginBottom: 14 }}>{t("ai.server")}</h2>
          <dl className={s.dl} style={{ gridTemplateColumns: "180px 1fr", fontSize: 15 }}>
            <dt>{t("ai.mode")}</dt><dd>{t(`ai.mode.${i.mode}`)}</dd>
            <dt>{t("ai.scale")}</dt><dd>{t(`ai.scale.${i.grading_scale}`)}</dd>
            <dt>{t("ai.domains")}</dt><dd>{i.allowed_domains.length ? i.allowed_domains.join(", ") : t("ai.noDomains")}</dd>
          </dl>
        </section>
      </div>
    </>
  );
}
