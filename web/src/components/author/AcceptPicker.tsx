"use client";

import { useState } from "react";
import { Icon } from "@/components/Icon";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import { ACCEPT_GROUPS, MENU_EXTS } from "@/lib/fileAssignment";

/** The file types a student may hand in, as groups of checkboxes. `value` holds menu extensions only. A group can be folded away. */
export function AcceptPicker({ value, onChange }: { value: string[]; onChange: (next: string[]) => void }) {
  const t = useT();
  // Groups that already have a pick start open, so what is chosen is never hidden.
  const [open, setOpen] = useState<MessageKey[]>(() => ACCEPT_GROUPS.filter((g) => g.types.some((x) => x.exts.some((e) => value.includes(e)))).map((g) => g.title));
  const set = (exts: string[], on: boolean) => {
    const next = new Set(value);
    for (const e of exts) (on ? next.add(e) : next.delete(e));
    onChange(MENU_EXTS.filter((e) => next.has(e)));
  };
  const toggle = (title: MessageKey, on: boolean) => setOpen((o) => (on ? [...o, title] : o.filter((x) => x !== title)));
  return (
    <div className="stack" style={{ gap: 8 }}>
      {ACCEPT_GROUPS.map((g) => {
        const all = g.types.flatMap((x) => x.exts);
        const picked = all.filter((e) => value.includes(e)).length;
        const shown = open.includes(g.title);
        const name = t(g.title);
        return (
          <div key={g.title} role="group" aria-label={name} className="stack" style={{ gap: 4 }}>
            <div className="row" style={{ justifyContent: "space-between", gap: 8 }}>
              <label className="check" style={{ minHeight: 32, fontWeight: 500 }}>
                <input
                  type="checkbox"
                  checked={picked === all.length}
                  ref={(el) => { if (el) el.indeterminate = picked > 0 && picked < all.length; }}
                  onChange={(e) => { set(all, e.target.checked); if (e.target.checked && !shown) toggle(g.title, true); }}
                />
                {name}
              </label>
              <button type="button" className="btn" style={{ minHeight: 32, gap: 4, padding: "0 8px", background: "transparent" }} aria-expanded={shown} aria-label={t("as.acceptToggle", { group: name })} onClick={() => toggle(g.title, !shown)}>
                {picked > 0 && <span className="muted" style={{ fontSize: 13 }}>{picked}/{all.length}</span>}
                <Icon name={shown ? "chevronUp" : "chevronDown"} size={16} />
              </button>
            </div>
            {shown && (
              <div className="chips">
                {g.types.map((x) => (
                  <label key={x.label} className="chip">
                    <input type="checkbox" className="sr-only" checked={x.exts.every((e) => value.includes(e))} onChange={(e) => set(x.exts, e.target.checked)} />
                    {x.label}
                  </label>
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
