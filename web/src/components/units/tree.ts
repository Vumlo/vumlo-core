import type { UnitSummary } from "@/lib/api/hooks/units";

export type Row = { unit: UnitSummary; level: number };

/** The flat list as a tree: each unit under its parent, by name, with its depth. */
export function rows(all: UnitSummary[]): Row[] {
  const known = new Set(all.map((u) => u.id));
  const children = new Map<string | null, UnitSummary[]>();
  for (const u of all) {
    const parent = u.parent_id && known.has(u.parent_id) ? u.parent_id : null;
    children.set(parent, [...(children.get(parent) ?? []), u]);
  }
  const out: Row[] = [];
  const walk = (parent: string | null, level: number) => {
    for (const unit of children.get(parent) ?? []) {
      out.push({ unit, level });
      walk(unit.id, level + 1);
    }
  };
  walk(null, 1);
  return out;
}
