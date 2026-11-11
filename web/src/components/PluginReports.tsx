"use client";

import { useId } from "react";
import { LoadFailed } from "@/components/StateScreen";
import { isDesktop } from "@/lib/api/client";
import { usePluginReports, type ReportSubject } from "@/lib/api/hooks/plugins";
import type { PluginReport } from "@/lib/api/types";
import { dateTime } from "@/lib/format";
import { useT } from "@/lib/i18n";
import { locale } from "@/lib/i18n/core";
import { clip, formatCell, MAX_SUMMARY, parseDisplay, rawJson } from "@/lib/pluginReport";
import s from "./PluginReports.module.css";

/**
 * What the installed plugins reported about a submitted attempt, an assignment or a course (ADR-0077), for staff.
 * The caller mounts it only for people who may read the reports; the server decides in the end (a 403 shows nothing).
 * Every string is rendered as text. Display convention v1: `lib/pluginReport.ts`.
 */
export function PluginReports({ subject, id }: { subject: ReportSubject; id: string }) {
  const t = useT();
  const heading = useId();
  const reports = usePluginReports(subject, id);
  // The server refuses what this person may not see; the panel then is not there, as the page would not be.
  if (reports.error && (reports.error.status === 403 || reports.error.status === 404)) return null;
  const items = reports.data?.items;
  return (
    <section className={s.panel} aria-labelledby={heading} data-subject={subject}>
      <h2 id={heading} className={s.heading}>{t("plugin.reports.title")}</h2>
      {reports.error && !items ? (
        <LoadFailed title={t("plugin.reports.failed")} />
      ) : !items ? (
        <p className="loading" role="status">{t("common.loading")}</p>
      ) : items.length === 0 ? (
        <p className="muted">{t("plugin.reports.empty")}</p>
      ) : (
        <ul className={s.list}>
          {items.map((r) => (
            <li key={`${r.plugin_id}/${r.key}`}><Report report={r} /></li>
          ))}
        </ul>
      )}
    </section>
  );
}

function Report({ report }: { report: PluginReport }) {
  const t = useT();
  const titleId = useId();
  const display = parseDisplay(report.plugin_id, report.data);
  const title = display?.title ?? t("plugin.reports.defaultTitle", { plugin: report.plugin_id, key: report.key });
  const lang = locale();
  // The desktop app is not on the origin of the core, so a path of the plugin does not lead anywhere there.
  const links = isDesktop() ? [] : (display?.links ?? []);
  const raw = display ? "" : isEmpty(report.data) ? "" : rawJson(report.data);

  return (
    <article className={s.report} aria-labelledby={titleId}>
      <div className={s.head}>
        <h3 id={titleId} className={s.title}>{title}</h3>
        {display && <span className={s.badge} data-level={display.level}>{t(`plugin.reports.level.${display.level}`)}</span>}
      </div>
      <p className={s.meta}>{t("plugin.reports.meta", { plugin: report.plugin_id, key: report.key, time: dateTime(report.updated_at) })}</p>
      <p className={s.summary}>{typeof report.summary === "string" ? clip(report.summary, MAX_SUMMARY) : ""}</p>

      {display && display.facts.length > 0 && (
        <dl className={s.facts} aria-label={t("plugin.reports.facts")}>
          {display.facts.map((f, i) => (
            <div key={i} className={s.fact}>
              <dt>{f.label}</dt>
              <dd>{formatCell(f.value, lang)}</dd>
            </div>
          ))}
        </dl>
      )}

      {display?.table && (
        <>
          {/* Scrolls sideways on a narrow screen; focusable so that the keyboard can scroll it. */}
          <div className={s.tableWrap} role="region" aria-label={t("plugin.reports.table", { title })} tabIndex={0}>
            <table className="table">
              <caption className="sr-only">{title}</caption>
              <thead>
                <tr>
                  {display.table.columns.map((c, i) => (
                    <th key={i} scope="col" className={numeric(display.table!.rows, i) ? s.num : undefined}>{c}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {display.table.rows.map((row, ri) => (
                  <tr key={ri}>
                    {row.map((cell, ci) =>
                      ci === 0 ? (
                        <th key={ci} scope="row" className={typeof cell === "number" ? s.num : undefined}>{formatCell(cell, lang)}</th>
                      ) : (
                        <td key={ci} className={numeric(display.table!.rows, ci) ? s.num : undefined}>{formatCell(cell, lang)}</td>
                      ),
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {display.table.hidden > 0 && (
            <p className={s.note}>{t("plugin.reports.rowsShown", { shown: formatCell(display.table.rows.length, lang), total: formatCell(display.table.rows.length + display.table.hidden, lang) })}</p>
          )}
        </>
      )}

      {links.length > 0 && (
        <ul className={s.links} aria-label={t("plugin.reports.links")}>
          {links.map((l, i) => (
            // A plain anchor: the browser follows it with the session cookie, and the core checks who asks (ADR-0078).
            <li key={i}><a className="btn btn-outline" href={l.href}>{l.title}</a></li>
          ))}
        </ul>
      )}

      {raw && (
        <details className={s.raw}>
          <summary>{t("plugin.reports.raw")}</summary>
          <pre>{raw}</pre>
        </details>
      )}
    </article>
  );
}

const numeric = (rows: (string | number)[][], col: number) => rows.length > 0 && rows.every((r) => typeof r[col] === "number");
const isEmpty = (data: unknown) => typeof data === "object" && data !== null && Object.keys(data).length === 0;
