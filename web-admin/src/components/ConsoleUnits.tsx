import { useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { errorText } from "@/lib/api/client";
import { useT } from "@/lib/i18n";
import { useDeleteUnit, useUnits, type UnitSummary } from "@/lib/api/hooks/units";
import { RoleAbilities } from "./RoleAbilities";
import { MoveUnit, NewUnit, RenameUnit } from "@/components/units/UnitDialogs";
import { UnitRoles } from "@/components/units/UnitRoles";
import { rows } from "@/components/units/tree";
import s from "./Admin.module.css";

type Dialogs = "new-top" | "new-inside" | "rename" | "move" | "delete" | null;

function DeleteUnit({ unit, onClose, onDeleted }: { unit: UnitSummary; onClose: () => void; onDeleted: () => void }) {
  const t = useT();
  const remove = useDeleteUnit(unit.id);
  return (
    <ConfirmDialog
      title={t("un.deleteTitle", { name: unit.name })}
      confirm={t("un.delete")}
      danger
      pending={remove.pending}
      error={errorText(remove.error)}
      onClose={() => { remove.reset(); onClose(); }}
      onConfirm={async () => { if (await remove.run()) onDeleted(); }}
    >
      {t("un.deleteBody")}
    </ConfirmDialog>
  );
}

/** The units of the institution, who holds a role on each, and what each role may do (ADR-0094, ADR-0095). */
export function ConsoleUnits() {
  const t = useT();
  const units = useUnits();
  const [selected, setSelected] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialogs>(null);
  const all = units.data;
  const tree = useMemo(() => (all ? rows(all) : []), [all]);
  const unit = all?.find((u) => u.id === selected) ?? null;
  const close = () => setDialog(null);

  return (
    <>
      <div className={s.head}>
        <div><h1>{t("un.title")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("un.lead")}</p></div>
      </div>
      <div className="stack" style={{ gap: 20 }}>
        <RoleAbilities />
        <div className={s.split}>
          <section className={`card ${s.section}`} style={{ flex: "1 1 300px", maxWidth: 420 }}>
            <div className="row" style={{ justifyContent: "space-between", marginBottom: 12, gap: 8 }}>
              <h2 id="un-h" style={{ fontSize: 18, fontWeight: 600 }}>{t("un.tree")}</h2>
              <button type="button" className="btn" onClick={() => setDialog("new-top")}>{t("un.newTop")}</button>
            </div>
            {!all ? (
              <p className="loading" role="status">{t("common.loading")}</p>
            ) : tree.length === 0 ? (
              <p className="muted">{t("un.none")}</p>
            ) : (
              <ul role="tree" aria-label={t("un.tree")} className={s.tree}>
                {tree.map(({ unit: u, level }) => (
                  <li key={u.id} role="none">
                    <button type="button" role="treeitem" aria-level={level} aria-selected={u.id === selected} className={s.treeItem} style={{ paddingLeft: 12 + (level - 1) * 20 }} onClick={() => setSelected(u.id)}>
                      {u.name}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
          <div className="stack" style={{ flex: "999 1 420px", minWidth: 0, gap: 20 }}>
            {unit ? (
              <>
                <div className="row" style={{ flexWrap: "wrap", gap: 8 }}>
                  <button type="button" className="btn" onClick={() => setDialog("new-inside")}>{t("un.newInside")}</button>
                  <button type="button" className="btn" onClick={() => setDialog("rename")}>{t("un.rename")}</button>
                  <button type="button" className="btn" onClick={() => setDialog("move")}>{t("un.move")}</button>
                  <button type="button" className="btn btn-danger" onClick={() => setDialog("delete")}>{t("un.delete")}</button>
                </div>
                <UnitRoles key={unit.id} unit={unit} />
              </>
            ) : (
              <p className="muted">{t("un.pick")}</p>
            )}
          </div>
        </div>
      </div>
      {dialog === "new-top" && <NewUnit inside={null} onClose={close} />}
      {dialog === "new-inside" && unit && <NewUnit inside={unit} onClose={close} />}
      {dialog === "rename" && unit && <RenameUnit unit={unit} onClose={close} />}
      {dialog === "move" && unit && all && <MoveUnit unit={unit} all={all} onClose={close} />}
      {dialog === "delete" && unit && <DeleteUnit unit={unit} onClose={close} onDeleted={() => { close(); setSelected(null); }} />}
    </>
  );
}
