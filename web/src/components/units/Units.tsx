"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { errorText } from "@/lib/api/client";
import { useDeleteUnit, useUnitCourses, useUnits, type UnitSummary } from "@/lib/api/hooks/units";
import { useT } from "@/lib/i18n";
import { courseHref } from "@/lib/routes";
import { MoveUnit, NewUnit, RenameUnit } from "./UnitDialogs";
import { UnitRoles } from "./UnitRoles";
import { rows } from "./tree";
import s from "./Units.module.css";

type Dialogs = "new" | "rename" | "move" | "delete" | null;

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

/** The units a person holds a role on, and the courses in them; with `manage_units` also the subunits and roles (ADR-0094). */
export function Units() {
  const t = useT();
  const units = useUnits();
  const [selected, setSelected] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialogs>(null);
  const all = units.data;
  const tree = useMemo(() => (all ? rows(all) : []), [all]);
  const unit = all?.find((u) => u.id === selected) ?? null;
  const courses = useUnitCourses(unit?.id ?? null);
  const manage = unit?.abilities.includes("manage_units") ?? false;
  // The server refuses a move outside the units the person manages.
  const targets = useMemo(() => (all ?? []).filter((u) => u.abilities.includes("manage_units")), [all]);
  const close = () => setDialog(null);

  return (
    <div className="container">
      <div className="page-head">
        <div>
          <h1 className="page-title">{t("nav.units")}</h1>
          <p className="muted" style={{ marginTop: 4 }}>{t("un.mine")}</p>
        </div>
      </div>
      <div className={s.split}>
        <section className={`card ${s.section}`} style={{ flex: "1 1 300px", maxWidth: 420 }}>
          <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 12 }}>{t("un.tree")}</h2>
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
              {manage && (
                <div className="row" style={{ flexWrap: "wrap", gap: 8 }}>
                  <button type="button" className="btn" onClick={() => setDialog("new")}>{t("un.newInside")}</button>
                  <button type="button" className="btn" onClick={() => setDialog("rename")}>{t("un.rename")}</button>
                  <button type="button" className="btn" onClick={() => setDialog("move")}>{t("un.move")}</button>
                  <button type="button" className="btn btn-danger" onClick={() => setDialog("delete")}>{t("un.delete")}</button>
                </div>
              )}
              <section aria-label={t("un.courses", { name: unit.name })} className={`card ${s.section}`}>
                <h2 style={{ fontSize: 18, fontWeight: 600 }}>{t("un.courses", { name: unit.name })}</h2>
                {!courses.data ? (
                  <p className="loading" role="status">{t("common.loading")}</p>
                ) : courses.data.length === 0 ? (
                  <p className="muted" style={{ marginTop: 8 }}>{t("un.noCourses")}</p>
                ) : (
                  <ul className={s.courseList}>
                    {courses.data.map((c) => <li key={c.id}><Link href={courseHref(c.id)}>{c.title}</Link></li>)}
                  </ul>
                )}
              </section>
              {manage && <UnitRoles key={unit.id} unit={unit} />}
            </>
          ) : (
            <p className="muted">{t("un.pickCourses")}</p>
          )}
        </div>
      </div>
      {dialog === "new" && unit && <NewUnit inside={unit} onClose={close} />}
      {dialog === "rename" && unit && <RenameUnit unit={unit} onClose={close} />}
      {dialog === "move" && unit && <MoveUnit unit={unit} all={targets} canTop={false} onClose={close} />}
      {dialog === "delete" && unit && <DeleteUnit unit={unit} onClose={close} onDeleted={() => { close(); setSelected(null); }} />}
    </div>
  );
}
