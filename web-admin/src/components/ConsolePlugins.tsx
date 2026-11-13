
import Link from "next/link";
import { StatusPill } from "@/components/StatusPill";
import { useAdminPlugins } from "@console/hooks/plugins-admin";
import { useT } from "@/lib/i18n";
import s from "./Admin.module.css";

/** The installed plugins: what runs, what is switched off and what still needs a value (ADR-0095). */
export function ConsolePlugins() {
  const t = useT();
  const plugins = useAdminPlugins();
  const items = plugins.data?.items;
  return (
    <>
      <div className={s.head}><div><h1>{t("pl.title")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("pl.lead")}</p></div></div>
      {!items ? (
        <p className="loading" role="status">{t("common.loading")}</p>
      ) : items.length === 0 ? (
        <p className="muted">{t("pl.none")}</p>
      ) : (
        <div className={`card ${s.tableCard}`}>
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{t("pl.col.plugin")}</th>
                  <th scope="col">{t("pl.col.status")}</th>
                  <th scope="col">{t("pl.col.setup")}</th>
                  <th scope="col">{t("pl.col.deliveries")}</th>
                </tr>
              </thead>
              <tbody>
                {items.map((p) => (
                  <tr key={p.plugin_id}>
                    <th scope="row">
                      <Link href={`/plugins/${encodeURIComponent(p.plugin_id)}`} style={{ fontWeight: 600 }}>{p.plugin_id}</Link>
                      <div className="muted" style={{ fontSize: 13, fontWeight: 400 }}>{t("pl.version", { version: p.version })}</div>
                    </th>
                    <td><StatusPill tone={p.status === "enabled" ? "ok" : "plain"}>{t(p.status === "enabled" ? "pl.enabled" : "pl.disabled")}</StatusPill></td>
                    <td>{p.needs_setup.length > 0 ? t("pl.needsSetup", { keys: p.needs_setup.join(", ") }) : p.status === "enabled" ? t("pl.ready") : null}</td>
                    <td>{t("pl.deliveries", { pending: p.pending, failed: p.failed })}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </>
  );
}
