"use client";

import { useState } from "react";
import { Icon } from "@/components/Icon";
import { useAuditLog } from "@/lib/api/hooks/admin";
import type { AuditKind } from "@/lib/api/types";
import { auditLine } from "@/lib/audit";
import { dayKey, dayLabel, relativeCap, timeOf, parts, dateTime, weekdayShort } from "@/lib/format";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import s from "./Admin.module.css";

const KINDS: [AuditKind | "all", MessageKey][] = [["all", "au.all"], ["accounts", "au.accounts"], ["owners", "au.owners"], ["teachers", "ap.teachers"], ["groups", "ag.groups"], ["courses", "au.kCourses"], ["content", "au.kContent"], ["grades", "au.kGrades"], ["terms", "au.kTerms"], ["other", "au.kOther"]];
const KIND_LABEL: Record<AuditKind, MessageKey> = { accounts: "au.kAccounts", owners: "au.kOwners", teachers: "ap.teachers", groups: "ag.groups", courses: "au.kCourses", content: "au.kContent", grades: "au.kGrades", terms: "au.kTerms", other: "au.kOther" };

export function AdminAudit() {
  const t = useT();
  const [q, setQ] = useState("");
  const [kind, setKind] = useState<AuditKind | "all">("all");
  const [days, setDays] = useState(7);
  const log = useAuditLog({ q, kind, days });
  const events = log.data ?? [];
  const byDay = [...new Set(events.map((e) => dayKey(e.at)))].map((k) => ({ key: k, items: events.filter((e) => dayKey(e.at) === k) }));

  function download() {
    const rows = [[t("au.time"), t("au.who"), t("au.what"), t("au.detail"), t("au.kind")], ...events.map((e) => { const l = auditLine(e); return [dateTime(e.at), l.who, l.what, l.detail, t(KIND_LABEL[e.kind])]; })];
    const text = rows.map((r) => r.map((c) => `"${c.replace(/"/g, '""')}"`).join(",")).join("\n");
    const url = URL.createObjectURL(new Blob([text], { type: "text/csv" }));
    const a = document.createElement("a");
    a.href = url; a.download = "audit-log.csv"; a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <>
      <div className={s.head}>
        <div><h1>{t("au.title")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("au.lead")}</p></div>
        <button type="button" className="btn" onClick={download} disabled={events.length === 0}><Icon name="download" size={16} strokeWidth={2} />{t("fb.csv")}</button>
      </div>
      <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 16 }}>
        <label className="search" style={{ flex: "0 1 260px", background: "var(--surface)" }}><Icon name="search" size={16} /><input type="search" aria-label={t("au.person")} placeholder={t("au.personShort")} value={q} onChange={(e) => setQ(e.target.value)} /></label>
        <select aria-label={t("au.kindOf")} className="select" style={{ width: "auto", minHeight: 40 }} value={kind} onChange={(e) => setKind(e.target.value as typeof kind)}>{KINDS.map(([k, l]) => <option key={k} value={k}>{t(l)}</option>)}</select>
        <select aria-label={t("au.period")} className="select" style={{ width: "auto", minHeight: 40 }} value={days} onChange={(e) => setDays(Number(e.target.value))}><option value={7}>{t("au.last7")}</option><option value={30}>{t("au.last30")}</option><option value={120}>{t("au.thisTerm")}</option></select>
      </div>
      {!log.data ? <p className="loading" role="status">{t("common.loading")}</p> : byDay.length === 0 ? <p className="empty">{t("au.none")}</p> : byDay.map((d) => {
        const p = parts(d.items[0]!.at);
        const label = `${relativeCap(d.items[0]!.at) === t("date.today.cap") ? `${t("date.today.cap")}, ` : ""}${weekdayShort(p.wd)} ${dayLabel(d.items[0]!.at)}`;
        return (
          <section key={d.key} aria-label={label} style={{ marginBottom: 22 }}>
            <h2 style={{ fontSize: 16, fontWeight: 600, margin: "0 2px 8px" }}>{label}</h2>
            <ol className="card" style={{ overflow: "hidden" }}>
              {d.items.map((e) => (
                <li key={e.id} className={s.ev}>
                  <span className={s.time}>{timeOf(e.at)}</span>
                  <span style={{ flex: "1 1 auto", fontSize: 15 }}><strong style={{ fontWeight: 600 }}>{auditLine(e).who}</strong> {auditLine(e).what}<span className="muted" style={{ display: "block", fontSize: 13 }}>{auditLine(e).detail}</span></span>
                  <span className={s.kind}>{t(KIND_LABEL[e.kind])}</span>
                </li>
              ))}
            </ol>
          </section>
        );
      })}
    </>
  );
}
