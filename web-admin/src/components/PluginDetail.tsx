
import Link from "next/link";
import { useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { StatusPill } from "@/components/StatusPill";
import { errorText } from "@/lib/api/client";
import { useAdminPlugin, useAdminPluginCalls, useAdminPluginSettings, useGrantPlugin, useSetPluginStatus } from "@console/hooks/plugins-admin";
import type { AdminPlugin } from "@/lib/api/types";
import { useT } from "@/lib/i18n";
import { SettingField } from "./SettingField";
import s from "./Admin.module.css";

function Access({ plugin }: { plugin: AdminPlugin }) {
  const t = useT();
  const grant = useGrantPlugin(plugin.plugin_id);
  const [scopes, setScopes] = useState(plugin.scopes);
  const [all, setAll] = useState(plugin.all_courses);
  const [saved, setSaved] = useState(false);

  async function save() {
    setSaved(false);
    if (await grant.run(scopes, all, plugin.course_ids)) setSaved(true);
  }
  const toggle = (scope: string, on: boolean) => {
    setSaved(false);
    setScopes((now) => (on ? [...now, scope] : now.filter((x) => x !== scope)));
  };
  return (
    <section aria-labelledby="acc-h" className={`card ${s.section}`}>
      <h2 id="acc-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("pl.access")}</h2>
      <p className="muted" style={{ margin: "4px 0 0", fontSize: 14 }}>{t("pl.accessLead", { name: plugin.plugin_id })}</p>
      <div className={s.scopes}>
        {plugin.requested_scopes.map((scope) => (
          <label key={scope}>
            <input type="checkbox" checked={scopes.includes(scope)} onChange={(e) => toggle(scope, e.target.checked)} />
            {scope}
          </label>
        ))}
      </div>
      <p style={{ fontWeight: 500, margin: "12px 0 4px" }}>{t("pl.courses")}</p>
      <label style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <input type="checkbox" checked={all} onChange={(e) => { setSaved(false); setAll(e.target.checked); }} />
        {t("pl.allCourses")}
      </label>
      {!all && plugin.course_ids.length > 0 && <p className="muted" style={{ fontSize: 14, marginTop: 6 }}>{t("pl.someCourses", { n: plugin.course_ids.length })}</p>}
      <div className="row" style={{ marginTop: 16, gap: 12, alignItems: "center" }}>
        <button type="button" className="btn btn-primary" disabled={grant.pending} onClick={() => void save()}>{t("pl.saveAccess")}</button>
        {saved && <span role="status" className="muted">{t("pl.saved")}</span>}
      </div>
      {grant.error && <p role="alert" className="field-error" style={{ marginTop: 8 }}>{errorText(grant.error)}</p>}
    </section>
  );
}

function Settings({ id }: { id: string }) {
  const t = useT();
  const settings = useAdminPluginSettings(id);
  return (
    <section aria-labelledby="set-h" className={`card ${s.section}`}>
      <h2 id="set-h" style={{ fontSize: 18, fontWeight: 600, marginBottom: 16 }}>{t("pl.settings")}</h2>
      {!settings.data ? (
        <p className="loading" role="status">{t("common.loading")}</p>
      ) : settings.data.items.length === 0 ? (
        <p className="muted">{t("pl.settingsNone")}</p>
      ) : (
        settings.data.items.map((setting) => <SettingField key={`${setting.key}:${setting.is_set}`} plugin={id} setting={setting} />)
      )}
    </section>
  );
}

function Calls({ id }: { id: string }) {
  const t = useT();
  const calls = useAdminPluginCalls(id);
  const counts = calls.data?.counts ?? [];
  return (
    <section aria-labelledby="call-h" className={`card ${s.section}`}>
      <h2 id="call-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("pl.calls")}</h2>
      <p className="muted" style={{ margin: "4px 0 12px", fontSize: 14 }}>{t("pl.callsLead")}</p>
      {!calls.data ? (
        <p className="loading" role="status">{t("common.loading")}</p>
      ) : counts.length === 0 ? (
        <p className="muted">{t("pl.noCalls")}</p>
      ) : (
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">{t("pl.col.route")}</th>
                <th scope="col">{t("pl.col.answer")}</th>
                <th scope="col">{t("pl.col.count")}</th>
                <th scope="col">{t("pl.col.avg")}</th>
              </tr>
            </thead>
            <tbody>
              {counts.map((c) => (
                <tr key={`${c.method} ${c.route} ${c.status}`}>
                  <th scope="row" style={{ fontWeight: 500 }}>{c.method} {c.route}</th>
                  <td>{c.status}</td>
                  <td>{c.calls}</td>
                  <td>{c.average_ms} ms</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

/** One plugin: its state, what it may reach, its settings and what it did to the API (ADR-0095). */
export function PluginDetail({ id }: { id: string }) {
  const t = useT();
  const plugin = useAdminPlugin(id);
  const status = useSetPluginStatus(id);
  const [asking, setAsking] = useState(false);
  const p = plugin.data;
  if (plugin.error) return <p role="alert" className="error-box">{errorText(plugin.error)}</p>;
  if (!p) return <p className="loading" role="status">{t("common.loading")}</p>;
  const enabled = p.status === "enabled";

  async function change() {
    if (await status.run(enabled ? "disabled" : "enabled")) setAsking(false);
  }
  return (
    <>
      <Link href="/plugins" className="muted" style={{ display: "inline-flex", alignItems: "center", gap: 6, marginBottom: 12 }}>
        <span aria-hidden="true">←</span>{t("pl.back")}
      </Link>
      <div className={s.head}>
        <div>
          <h1>{p.plugin_id}</h1>
          <p className="muted" style={{ marginTop: 4 }}>{t("pl.version", { version: p.version })} · {p.base_url}</p>
        </div>
        <span data-testid="status"><StatusPill tone={enabled ? "ok" : "plain"}>{t(enabled ? "pl.enabled" : "pl.disabled")}</StatusPill></span>
        <button type="button" className={`btn ${enabled ? "" : "btn-primary"}`} onClick={() => (enabled ? setAsking(true) : void change())} disabled={status.pending}>
          {t(enabled ? "pl.disable" : "pl.enable")}
        </button>
      </div>
      {!asking && status.error && <p role="alert" className="field-error" style={{ marginBottom: 12 }}>{errorText(status.error) ?? t("pl.statusFailed")}</p>}
      <div className="stack" style={{ gap: 20 }}>
        <Access plugin={p} />
        <Settings id={p.plugin_id} />
        <Calls id={p.plugin_id} />
      </div>
      {asking && (
        <ConfirmDialog
          title={t("pl.disableTitle", { name: p.plugin_id })}
          confirm={t("pl.disable")}
          danger
          pending={status.pending}
          error={errorText(status.error)}
          onClose={() => { setAsking(false); status.reset(); }}
          onConfirm={() => void change()}
        >
          {t("pl.disableBody")}
        </ConfirmDialog>
      )}
    </>
  );
}
