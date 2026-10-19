"use client";

import Link from "next/link";
import { useState } from "react";
import { Icon } from "@/components/Icon";
import { useInstitution } from "@/lib/api/hooks/institution";
import { SYSTEMS, useAppOffer } from "@/lib/downloads";
import { useT } from "@/lib/i18n";
import s from "./App.module.css";

const KEY = "vumlo.appIsland.hidden";

function hidden() {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * A small notice on the home page: the institution has a desktop app (ADR-0064). It shows only when an admin set
 * at least one link, on a computer the app is made for, and until the person hides it; the download page and the
 * account menu stay.
 */
export function AppIsland() {
  const t = useT();
  const institution = useInstitution();
  const { links, system, direct } = useAppOffer();
  const [gone, setGone] = useState(hidden);
  if (!links.length || !system || gone) return null;
  const label = SYSTEMS.find((entry) => entry.id === system);
  const name = institution.data?.short_name ?? "";
  return (
    <div className="container" style={{ paddingBottom: 0 }}>
      <section aria-label={t("app.island.title", { name })} className={s.island}>
        <span aria-hidden="true" className={s.islandIcon}><Icon name="download" size={20} /></span>
        <div className={s.islandText}>
          <p className={s.islandTitle}>{t("app.island.title", { name })}</p>
          <p className={s.islandLead}>{t("app.island.lead")}</p>
        </div>
        <div className={s.islandActions}>
          {direct ? (
            <a href={direct.url} className="btn btn-primary" rel="noopener">{t("app.island.download", { system: label ? t(label.label) : "" })}</a>
          ) : (
            <Link href="/download" className="btn btn-primary">{t("app.island.get")}</Link>
          )}
          {direct && <Link href="/download" className="btn btn-quiet">{t("app.island.all")}</Link>}
          <button
            type="button"
            className="icon-btn"
            aria-label={t("app.island.hide")}
            title={t("app.island.hide")}
            onClick={() => {
              try {
                localStorage.setItem(KEY, "1");
              } catch {
                // Storage can be blocked; the notice then comes back next time.
              }
              setGone(true);
            }}
          >
            <Icon name="close" size={18} />
          </button>
        </div>
      </section>
    </div>
  );
}
