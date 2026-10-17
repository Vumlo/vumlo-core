"use client";

import { useState } from "react";
import { Icon } from "@/components/Icon";
import { useAuditLog } from "@/lib/api/hooks/admin";
import type { AuditKind } from "@/lib/api/gaps";
import { dayKey, dayLabel, relativeCap, timeOf, parts, dateTime } from "@/lib/format";
import s from "./Admin.module.css";

const KINDS: [AuditKind | "all", string][] = [["all", "All changes"], ["accounts", "Accounts and roles"], ["owners", "Course owners and teachers"], ["groups", "Groups"]];
const KIND_LABEL: Record<AuditKind, string> = { accounts: "Accounts", owners: "Owners", teachers: "Teachers", groups: "Groups" };
const WD = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

export function AdminAudit() {
  const [q, setQ] = useState("");
  const [kind, setKind] = useState<AuditKind | "all">("all");
  const [days, setDays] = useState(7);
  const log = useAuditLog({ q, kind, days });
  const events = log.data ?? [];
  const byDay = [...new Set(events.map((e) => dayKey(e.at)))].map((k) => ({ key: k, items: events.filter((e) => dayKey(e.at) === k) }));

  function download() {
    const rows = [["Time", "Who", "What", "Detail", "Kind"], ...events.map((e) => [dateTime(e.at), e.who, e.what, e.detail, KIND_LABEL[e.kind]])];
    const text = rows.map((r) => r.map((c) => `"${c.replace(/"/g, '""')}"`).join(",")).join("\n");
    const url = URL.createObjectURL(new Blob([text], { type: "text/csv" }));
    const a = document.createElement("a");
    a.href = url; a.download = "audit-log.csv"; a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <>
      <div className={s.head}>
        <div><h1>Audit log</h1><p className="muted" style={{ marginTop: 4 }}>Changes to accounts, roles, course owners, teachers and groups. Nothing here can be edited or deleted.</p></div>
        <button type="button" className="btn" onClick={download} disabled={events.length === 0}><Icon name="download" size={16} strokeWidth={2} />Download CSV</button>
      </div>
      <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 16 }}>
        <label className="search" style={{ flex: "0 1 260px", background: "var(--surface)" }}><Icon name="search" size={16} /><input type="search" aria-label="Who did it or who it affected" placeholder="Person" value={q} onChange={(e) => setQ(e.target.value)} /></label>
        <select aria-label="Kind of change" className="select" style={{ width: "auto", minHeight: 40 }} value={kind} onChange={(e) => setKind(e.target.value as typeof kind)}>{KINDS.map(([k, l]) => <option key={k} value={k}>{l}</option>)}</select>
        <select aria-label="Period" className="select" style={{ width: "auto", minHeight: 40 }} value={days} onChange={(e) => setDays(Number(e.target.value))}><option value={7}>Last 7 days</option><option value={30}>Last 30 days</option><option value={120}>This term</option></select>
      </div>
      {!log.data ? <p className="loading" role="status">Loading…</p> : byDay.length === 0 ? <p className="empty">No changes match.</p> : byDay.map((d) => {
        const p = parts(d.items[0]!.at);
        const label = `${relativeCap(d.items[0]!.at) === "Today" ? "Today, " : ""}${WD[p.wd]} ${dayLabel(d.items[0]!.at)}`;
        return (
          <section key={d.key} aria-label={label} style={{ marginBottom: 22 }}>
            <h2 style={{ fontSize: 16, fontWeight: 600, margin: "0 2px 8px" }}>{label}</h2>
            <ol className="card" style={{ overflow: "hidden" }}>
              {d.items.map((e) => (
                <li key={e.id} className={s.ev}>
                  <span className={s.time}>{timeOf(e.at)}</span>
                  <span style={{ flex: "1 1 auto", fontSize: 15 }}><strong style={{ fontWeight: 600 }}>{e.who}</strong> {e.what}<span className="muted" style={{ display: "block", fontSize: 13 }}>{e.detail}</span></span>
                  <span className={s.kind}>{KIND_LABEL[e.kind]}</span>
                </li>
              ))}
            </ol>
          </section>
        );
      })}
    </>
  );
}
