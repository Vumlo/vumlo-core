import { useState } from "react";
import { errorText } from "@/lib/api/client";
import { useT } from "@/lib/i18n";
import { useRemoveUnitRole, useSetUnitRole, useUnitCandidates, useUnitRoles, type UnitRole, type UnitSummary } from "@/lib/api/hooks/units";
import s from "./Admin.module.css";

/** Who holds a role on a unit: it covers the unit and every unit below it (ADR-0094). */
export function UnitRoles({ unit }: { unit: UnitSummary }) {
  const t = useT();
  const roles = useUnitRoles(unit.id);
  const set = useSetUnitRole(unit.id);
  const remove = useRemoveUnitRole(unit.id);
  const [q, setQ] = useState("");
  const found = useUnitCandidates(unit.id, q);
  const label = t("un.roles", { name: unit.name });
  const failed = errorText(set.error ?? remove.error);

  return (
    <section aria-label={label} className={`card ${s.section}`}>
      <h2 style={{ fontSize: 18, fontWeight: 600 }}>{label}</h2>
      {!roles.data ? (
        <p className="loading" role="status">{t("common.loading")}</p>
      ) : roles.data.length === 0 ? (
        <p className="muted" style={{ margin: "8px 0" }}>{t("un.noRoles")}</p>
      ) : (
        <div className="table-wrap" style={{ margin: "12px 0" }}>
          <table className="table">
            <thead>
              <tr><th scope="col">{t("un.col.person")}</th><th scope="col">{t("un.col.role")}</th><th scope="col"><span className="sr-only">{t("un.col.actions")}</span></th></tr>
            </thead>
            <tbody>
              {roles.data.map((r) => (
                <tr key={r.user_id}>
                  <th scope="row" style={{ fontWeight: 500 }}>{r.name} {r.surname}<div className="muted" style={{ fontSize: 13, fontWeight: 400 }}>{r.email}</div></th>
                  <td>
                    <select className="input" aria-label={t("un.roleOf", { name: `${r.name} ${r.surname}` })} value={r.role} disabled={set.pending} onChange={(e) => void set.run(r.user_id, e.target.value as UnitRole)}>
                      <option value="manager">{t("un.role.manager")}</option>
                      <option value="viewer">{t("un.role.viewer")}</option>
                    </select>
                  </td>
                  <td><button type="button" className="btn" aria-label={t("un.remove", { name: `${r.name} ${r.surname}` })} disabled={remove.pending} onClick={() => void remove.run(r.user_id)}>{t("un.removeBtn")}</button></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {failed && <p role="alert" className="field-error">{failed}</p>}
      <div className="field" style={{ marginTop: 12 }}>
        <label htmlFor="unit-search">{t("un.addPerson")}</label>
        <input id="unit-search" type="search" className="input" aria-label={t("un.search")} placeholder={t("un.search")} value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      {q.trim() !== "" && found.data && (found.data.items.length === 0 ? (
        <p className="muted" style={{ marginTop: 8 }}>{t("un.noCandidates")}</p>
      ) : (
        <ul className={s.candidates}>
          {found.data.items.map((c) => (
            <li key={c.user_id}>
              <span>{c.name} {c.surname} <span className="muted">{c.email}</span></span>
              <button type="button" className="btn" aria-label={t("un.add", { name: `${c.name} ${c.surname}` })} disabled={set.pending} onClick={async () => { if (await set.run(c.user_id, "viewer")) setQ(""); }}>{t("un.addBtn")}</button>
            </li>
          ))}
        </ul>
      ))}
    </section>
  );
}
