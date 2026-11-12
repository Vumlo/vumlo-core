"use client";

import { useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { errorText } from "@/lib/api/client";
import { useUpdateItem, useUpdateModule } from "@/lib/api/hooks/content";
import type { AccessRule, ContentTree, RuleCondition } from "@/lib/api/types";
import { isAssignment, sections } from "@/lib/course";
import { useT } from "@/lib/i18n";

/** What a rule locks: an item or a module (ADR-0092). */
export type RuleTarget = { kind: "item" | "module"; id: string; title: string; rule?: AccessRule | null };

/** One line of the form: what is asked, of which item or module, and for a score the points. */
type Row = { what: "item" | "module" | "score"; id: string; points: string };

const toRow = (c: RuleCondition): Row =>
  c.kind === "score_at_least"
    ? { what: "score", id: c.item_id ?? "", points: c.points ?? "" }
    : c.module_id
    ? { what: "module", id: c.module_id, points: "" }
    : { what: "item", id: c.item_id ?? "", points: "" };

const toCondition = (r: Row): RuleCondition =>
  r.what === "score"
    ? { kind: "score_at_least", item_id: r.id, points: r.points.trim() }
    : r.what === "module"
    ? { kind: "completed", module_id: r.id }
    : { kind: "completed", item_id: r.id };

/** The teacher's form for the rule of one item or module: every condition must hold for a student to open it. */
export function AccessRuleDialog({ courseId, tree, target, onClose }: { courseId: string; tree: ContentTree; target: RuleTarget; onClose: () => void }) {
  const t = useT();
  const updateItem = useUpdateItem(courseId);
  const updateModule = useUpdateModule(courseId);
  const [rows, setRows] = useState<Row[]>((target.rule?.all ?? []).map(toRow));
  const pending = updateItem.pending || updateModule.pending;
  const error = updateItem.error ?? updateModule.error;

  // What a condition can name: the other items and sections of the course, and its assignments for a score.
  const secs = sections(tree).filter((sec) => !(target.kind === "module" && sec.id === target.id));
  const items = sections(tree).flatMap((sec) => sec.items.filter((i) => !(target.kind === "item" && i.id === target.id)).map((i) => ({ item: i, label: `${sec.title} / ${i.title}` })));
  const options = (what: Row["what"]) =>
    what === "module" ? secs.map((sec) => ({ id: sec.id, label: sec.title })) : items.filter(({ item }) => what === "item" || isAssignment(item)).map(({ item, label }) => ({ id: item.id, label }));

  const set = (index: number, change: Partial<Row>) => setRows((all) => all.map((row, at) => (at === index ? { ...row, ...change } : row)));

  async function save() {
    const rule: AccessRule | null = rows.length > 0 ? { all: rows.map(toCondition) } : null;
    const saved = target.kind === "item" ? await updateItem.run(target.id, { access_rule: rule }) : await updateModule.run(target.id, { access_rule: rule });
    if (saved) onClose();
  }

  return (
    <Dialog title={t("rule.title", { title: target.title })} description={t("rule.lead")} onClose={onClose} width={640}
      actions={<><button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" onClick={save} disabled={pending}>{t("cm.save")}</button></>}>
      <div className="stack" style={{ gap: 16 }}>
        {rows.length === 0 && <p className="muted">{t("rule.none")}</p>}
        {rows.map((row, index) => (
          <div key={index} className="row" style={{ gap: 8, flexWrap: "wrap", alignItems: "flex-end" }}>
            <div className="field">
              <label htmlFor={`rule-kind-${index}`}>{t("rule.kind")}</label>
              <select id={`rule-kind-${index}`} className="input" value={row.what} onChange={(e) => set(index, { what: e.target.value as Row["what"], id: "" })}>
                <option value="item">{t("rule.kind.item")}</option>
                <option value="module">{t("rule.kind.module")}</option>
                <option value="score">{t("rule.kind.score")}</option>
              </select>
            </div>
            <div className="field" style={{ flex: 1, minWidth: 180 }}>
              <label htmlFor={`rule-target-${index}`}>{t("rule.target")}</label>
              <select id={`rule-target-${index}`} className="input" value={row.id} onChange={(e) => set(index, { id: e.target.value })}>
                <option value="">{t("rule.pick")}</option>
                {options(row.what).map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}
              </select>
            </div>
            {row.what === "score" && (
              <div className="field">
                <label htmlFor={`rule-points-${index}`}>{t("rule.points")}</label>
                <input id={`rule-points-${index}`} className="input" inputMode="decimal" style={{ maxWidth: 100 }} value={row.points} onChange={(e) => set(index, { points: e.target.value })} />
              </div>
            )}
            <button type="button" className="icon-btn" aria-label={t("rule.remove")} onClick={() => setRows((all) => all.filter((_, at) => at !== index))}><Icon name="trash" size={16} /></button>
          </div>
        ))}
        <div>
          <button type="button" className="btn btn-outline" onClick={() => setRows((all) => [...all, { what: "item", id: "", points: "" }])}><Icon name="plus" size={15} strokeWidth={2.2} />{t("rule.add")}</button>
        </div>
        {error && <p role="alert" className="alert alert-danger">{errorText(error)}</p>}
      </div>
    </Dialog>
  );
}
