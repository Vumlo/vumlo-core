"use client";

import Link from "next/link";
import { useState, type FormEvent } from "react";
import { useSetAppDownloads } from "@/lib/api/hooks/admin";
import type { AppDownload, AppPlatform } from "@/lib/api/types";
import { SYSTEMS } from "@/lib/downloads";
import { useT } from "@/lib/i18n";
import app from "@/components/app/App.module.css";
import s from "./Admin.module.css";

const PLATFORMS = SYSTEMS.flatMap((system) => system.platforms);

/** The links to the institution's desktop app (ADR-0064): all five installers, any of them empty. */
export function AdminAppDownloads({ current }: { current: AppDownload[] }) {
  const t = useT();
  const save = useSetAppDownloads();
  const [urls, setUrls] = useState<Record<string, string>>(() => Object.fromEntries(current.map((link) => [link.platform, link.url])));
  const [saved, setSaved] = useState<number | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setSaved(null);
    const downloads = PLATFORMS.flatMap(({ platform }) => {
      const url = (urls[platform] ?? "").trim();
      return url ? [{ platform: platform as AppPlatform, url }] : [];
    });
    const result = await save.run(downloads);
    if (result) {
      setUrls(Object.fromEntries(result.map((link) => [link.platform, link.url])));
      setSaved(result.length);
    }
  }

  return (
    <section aria-labelledby="ap-h" className={`card ${s.section}`}>
      <h2 id="ap-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("ai.appTitle")}</h2>
      <p className="muted" style={{ margin: "4px 0 14px", fontSize: 14 }}>{t("ai.appLead")}</p>
      <form className={app.fields} onSubmit={submit}>
        {PLATFORMS.map(({ platform, label }) => (
          <div className="field" key={platform}>
            <label htmlFor={`dl-${platform}`}>{t(label)}</label>
            <input
              id={`dl-${platform}`}
              className="input"
              type="url"
              inputMode="url"
              autoComplete="off"
              placeholder="https://…"
              value={urls[platform] ?? ""}
              onChange={(e) => { setSaved(null); setUrls({ ...urls, [platform]: e.target.value }); }}
              aria-invalid={!!save.error}
            />
          </div>
        ))}
        {save.error && <p role="alert" className="alert alert-danger">{save.error.errors[0]?.message ? t("ai.appInvalid") : (save.error.detail ?? save.error.title)}</p>}
        {saved !== null && <p role="status" className="alert alert-ok">{saved > 0 ? t("ai.appSaved") : t("ai.appOff")}</p>}
        <div className="row" style={{ gap: 12, flexWrap: "wrap" }}>
          <button type="submit" className="btn btn-primary" disabled={save.pending}>{t("ai.appSave")}</button>
          <Link href="/download" className="btn btn-quiet">{t("ai.appOpen")}</Link>
        </div>
      </form>
    </section>
  );
}
