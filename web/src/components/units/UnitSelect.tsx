"use client";

import { useUnits } from "@/lib/api/hooks/units";
import { useT } from "@/lib/i18n";
import { rows } from "./tree";

/**
 * The unit a course sits in: "No unit" and the units where the person may place courses, indented by depth.
 * A unit the person cannot place into (`current`) is still shown, selected, so the screen tells the truth.
 * Empty `value` means no unit. Renders nothing when there is nothing to choose and nothing to show.
 */
export function UnitSelect({ id, value, onChange, current, error }: { id: string; value: string; onChange: (unitId: string) => void; current?: string | null; error?: string }) {
  const t = useT();
  const units = useUnits();
  const all = units.data ?? [];
  const placeable = rows(all).filter((r) => r.unit.abilities.includes("manage_courses"));
  const away = current && !placeable.some((r) => r.unit.id === current) ? current : null;
  if (placeable.length === 0 && !away) return null;
  return (
    <div className="field">
      <label htmlFor={id}>{t("un.unit")}</label>
      <select id={id} className="select" value={value} onChange={(e) => onChange(e.target.value)} aria-invalid={!!error}>
        <option value="">{t("un.noUnit")}</option>
        {away && <option value={away} disabled>{all.find((u) => u.id === away)?.name ?? t("un.elsewhere")}</option>}
        {placeable.map((r) => <option key={r.unit.id} value={r.unit.id}>{"  ".repeat(r.level - 1)}{r.unit.name}</option>)}
      </select>
      {error && <p className="field-error">{error}</p>}
    </div>
  );
}
