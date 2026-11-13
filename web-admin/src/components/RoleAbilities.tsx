import { useState } from "react";
import { errorText } from "@/lib/api/client";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import { useRoleAbilities, useSetRoleAbilities, type UnitAbility, type UnitRole } from "@console/hooks/units";
import s from "./Admin.module.css";

const ABILITIES: [UnitAbility, MessageKey][] = [
  ["view_grades", "un.ability.view_grades"],
  ["edit_content", "un.ability.edit_content"],
  ["grade", "un.ability.grade"],
  ["manage_roster", "un.ability.manage_roster"],
  ["manage_courses", "un.ability.manage_courses"],
  ["manage_units", "un.ability.manage_units"],
];

/**
 * The switches of the two unit roles. They are the same for every unit: changing one changes the role everywhere,
 * and the page says so (ADR-0094).
 */
export function RoleAbilities() {
  const t = useT();
  const abilities = useRoleAbilities();
  const managers = useSetRoleAbilities("manager");
  const viewers = useSetRoleAbilities("viewer");
  const [saved, setSaved] = useState(false);
  const data = abilities.data;
  if (!data) return <p className="loading" role="status">{t("common.loading")}</p>;
  const failed = errorText(managers.error ?? viewers.error);

  async function toggle(role: UnitRole, ability: UnitAbility, on: boolean) {
    setSaved(false);
    const held = data![role];
    const next = on ? [...held, ability] : held.filter((a) => a !== ability);
    if (await (role === "manager" ? managers : viewers).run(next)) setSaved(true);
  }
  return (
    <section aria-label={t("un.abilities")} className={`card ${s.section}`}>
      <h2 style={{ fontSize: 18, fontWeight: 600 }}>{t("un.abilities")}</h2>
      <p className="muted" style={{ margin: "4px 0 12px", fontSize: 14 }}>{t("un.abilitiesLead")}</p>
      <div className="table-wrap">
        <table className="table">
          <thead>
            <tr>
              <th scope="col"><span className="sr-only">{t("un.col.ability")}</span></th>
              <th scope="col">{t("un.role.manager")}</th>
              <th scope="col">{t("un.role.viewer")}</th>
            </tr>
          </thead>
          <tbody>
            {ABILITIES.map(([ability, name]) => (
              <tr key={ability}>
                <th scope="row" style={{ fontWeight: 500 }}>{t(name)}</th>
                {(["manager", "viewer"] as const).map((role) => (
                  <td key={role}>
                    <input
                      type="checkbox"
                      aria-label={`${t(role === "manager" ? "un.role.manager" : "un.role.viewer")}: ${t(name)}`}
                      checked={data[role].includes(ability)}
                      disabled={managers.pending || viewers.pending}
                      onChange={(e) => void toggle(role, ability, e.target.checked)}
                    />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p role="status" className="muted" style={{ marginTop: 8, minHeight: 22 }}>{saved ? t("un.abilitySaved") : null}</p>
      {failed && <p role="alert" className="field-error">{failed}</p>}
    </section>
  );
}
