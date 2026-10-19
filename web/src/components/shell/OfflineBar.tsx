"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { useLanguage, useT } from "@/lib/i18n";
import { timeOf } from "@/lib/format";
import { onNavigate, outboxDiscard, outboxList, prefetch, startSync, syncLanguage, syncNow, useSync, type OutboxItem } from "@/lib/offline";
import s from "./Shell.module.css";

const PREFETCH_EVERY_MS = 30 * 60 * 1000;

/** The words of a draft, for a person who wants to keep them after the server refused the draft. */
function textOf(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) return value.map(textOf).filter(Boolean).join("\n");
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return "text" in record && typeof record.text === "string" ? record.text : Object.values(record).map(textOf).filter(Boolean).join(record.type === "paragraph" ? "" : "\n");
  }
  return "";
}

function reasonOf(problem: string | null) {
  try {
    const p = JSON.parse(problem ?? "") as { detail?: string; title?: string };
    return p.detail ?? p.title ?? null;
  } catch {
    return null;
  }
}

function Failed({ item }: { item: OutboxItem }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const kind = item.path.endsWith("/submission/draft") ? "offline.item.draft" : item.path.endsWith("/submit") ? "offline.item.submit" : "offline.item.other";
  const reason = reasonOf(item.problem);
  let words = "";
  try {
    words = item.body ? textOf((JSON.parse(item.body) as { content?: unknown }).content) : "";
  } catch {
    words = "";
  }
  return (
    <li className="row" style={{ gap: 12, flexWrap: "wrap", alignItems: "baseline" }}>
      <strong>{t(kind)}</strong>
      <span style={{ flex: 1 }}>{reason ? t("offline.reason", { reason }) : t("offline.noReason")}</span>
      {words && (
        <button type="button" className="btn btn-quiet" onClick={async () => { await navigator.clipboard.writeText(words); setCopied(true); }}>
          {copied ? t("offline.copied") : t("offline.copy")}
        </button>
      )}
      <button type="button" className="btn btn-quiet" onClick={() => void outboxDiscard(item.id)}>{t("offline.discard")}</button>
    </li>
  );
}

/**
 * The desktop app's connection and queue, above every signed-in screen (ADR-0060): offline with the time of the
 * copy it shows, what waits to be sent, and what the server refused. It also keeps the copy warm.
 */
export function OfflineBar({ role }: { role: string }) {
  const t = useT();
  const router = useRouter();
  const { language } = useLanguage();
  const sync = useSync();
  const [open, setOpen] = useState(false);
  const [failed, setFailed] = useState<OutboxItem[]>([]);

  useEffect(() => {
    void startSync();
  }, []);
  useEffect(() => onNavigate((route) => router.push(route)), [router]);
  useEffect(() => {
    void syncLanguage(language);
  }, [language]);

  const ready = sync !== null;
  const online = sync?.online ?? false;
  const failures = sync?.failed ?? 0;
  useEffect(() => {
    // At start, whenever the connection comes back, and then every half hour.
    if (!ready || !online) return;
    void prefetch(role);
    const timer = setInterval(() => void prefetch(role), PREFETCH_EVERY_MS);
    return () => clearInterval(timer);
  }, [ready, online, role]);

  useEffect(() => {
    if (!open) return;
    void outboxList().then((rows) => setFailed(rows.filter((row) => row.state === "failed")));
  }, [open, failures]);

  if (!sync) return null;
  const time = sync.last_ok ? timeOf(new Date(sync.last_ok).toISOString()) : null;
  return (
    <div className={s.offline}>
      {!sync.online && (
        <p role="status" className="alert alert-info">
          <span style={{ flex: 1 }}>
            {t("offline.offline")} {time && t("offline.lastSync", { time })} {sync.pending > 0 && t.plural("offline.waiting", sync.pending)}
          </span>
          <button type="button" className="btn btn-quiet" onClick={() => void syncNow()}>{t("offline.tryNow")}</button>
        </p>
      )}
      {sync.online && sync.pending > 0 && !sync.paused && <p role="status" className="alert alert-info">{t("offline.sending")}</p>}
      {sync.paused && <p role="alert" className="alert alert-danger">{t("offline.paused")}</p>}
      {failures > 0 && (
        <div role="alert" className="alert alert-danger" style={{ display: "block" }}>
          <div className="row" style={{ gap: 12, justifyContent: "space-between" }}>
            <span>{t.plural("offline.failed", failures)}</span>
            <button type="button" className="btn btn-quiet" aria-expanded={open} onClick={() => setOpen(!open)}>{open ? t("offline.hide") : t("offline.review")}</button>
          </div>
          {open && <ul style={{ listStyle: "none", padding: 0, margin: "10px 0 0", display: "grid", gap: 8 }}>{failed.map((item) => <Failed key={item.id} item={item} />)}</ul>}
        </div>
      )}
    </div>
  );
}
