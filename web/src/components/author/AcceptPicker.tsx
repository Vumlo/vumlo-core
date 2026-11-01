"use client";

import { useT } from "@/lib/i18n";
import { ACCEPT_GROUPS, MENU_EXTS } from "@/lib/fileAssignment";

/** The file types a student may hand in, as groups of checkboxes. `value` holds menu extensions only. */
export function AcceptPicker({ value, onChange }: { value: string[]; onChange: (next: string[]) => void }) {
  const t = useT();
  const set = (exts: string[], on: boolean) => {
    const next = new Set(value);
    for (const e of exts) (on ? next.add(e) : next.delete(e));
    onChange(MENU_EXTS.filter((e) => next.has(e)));
  };
  return (
    <div className="stack" style={{ gap: 12 }}>
      {ACCEPT_GROUPS.map((g) => {
        const all = g.types.flatMap((x) => x.exts);
        const picked = all.filter((e) => value.includes(e)).length;
        return (
          <fieldset key={g.title} className="stack" style={{ gap: 8, border: 0, margin: 0, padding: 0, minWidth: 0 }}>
            <legend style={{ padding: 0, marginBottom: 8 }}>
              <label className="check" style={{ minHeight: 28, fontWeight: 500 }}>
                <input
                  type="checkbox"
                  checked={picked === all.length}
                  ref={(el) => { if (el) el.indeterminate = picked > 0 && picked < all.length; }}
                  onChange={(e) => set(all, e.target.checked)}
                />
                {t(g.title)}
              </label>
            </legend>
            <div className="chips">
              {g.types.map((x) => (
                <label key={x.label} className="chip">
                  <input type="checkbox" className="sr-only" checked={x.exts.every((e) => value.includes(e))} onChange={(e) => set(x.exts, e.target.checked)} />
                  {x.label}
                </label>
              ))}
            </div>
          </fieldset>
        );
      })}
    </div>
  );
}
