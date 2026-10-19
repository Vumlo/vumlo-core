"use client";

import Link from "next/link";
import { useInstitution } from "@/lib/api/hooks/institution";
import { SYSTEMS, useAppOffer } from "@/lib/downloads";
import { useT } from "@/lib/i18n";
import app from "@/components/app/App.module.css";
import s from "../auth.module.css";

/** Where the institution's desktop app is downloaded (ADR-0064). Public: the link can go on the university's own site. */
export default function DownloadPage() {
  const t = useT();
  const institution = useInstitution();
  const { links, system, loaded } = useAppOffer();
  const name = institution.data?.short_name ?? "";
  if (!loaded) return <p className="loading" role="status">{t("common.loading")}</p>;
  if (!links.length) {
    return (
      <div className={s.card}>
        <h1 className={s.title}>{t("app.page.noneTitle")}</h1>
        <p className={s.lead}>{t("app.page.noneLead")}</p>
        <Link href="/" className="btn btn-outline">{t("app.page.home")}</Link>
      </div>
    );
  }
  // The system of this computer first, and marked.
  const groups = SYSTEMS.map((entry) => ({ ...entry, offered: entry.platforms.filter((p) => links.some((link) => link.platform === p.platform)) }))
    .filter((entry) => entry.offered.length)
    .sort((a, b) => Number(b.id === system) - Number(a.id === system));
  return (
    <div className={s.card}>
      <h1 className={s.title}>{t("app.page.title", { name })}</h1>
      <p className={s.lead}>{t("app.page.lead")}</p>
      {groups.map((group) => (
        <section key={group.id} aria-labelledby={`os-${group.id}`} className={app.group}>
          <div className={app.groupHead}>
            <h2 id={`os-${group.id}`} className={app.groupName}>{t(group.label)}</h2>
            {group.id === system && <span className={app.yours}>{t("app.page.yours")}</span>}
          </div>
          {group.offered.map((p) => {
            const link = links.find((entry) => entry.platform === p.platform)!;
            return (
              <a key={p.platform} href={link.url} rel="noopener" className={`btn btn-lg btn-block ${group.id === system ? "btn-primary" : "btn-outline"}`}>
                {t(p.label)}
              </a>
            );
          })}
        </section>
      ))}
      <p className="muted" style={{ marginTop: 20, fontSize: 14 }}>{t("app.page.note")}</p>
    </div>
  );
}
