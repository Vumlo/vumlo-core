import { useState, type FormEvent } from "react";
import { Dialog } from "@/components/Dialog";
import { errorText } from "@/lib/api/client";
import { useT } from "@/lib/i18n";
import { useCreateUnit, useUpdateUnit, type UnitSummary } from "@console/hooks/units";

/** The form of one change to a unit: a name, and where it sits. */
function UnitForm({ title, submit, name: first, parent: firstParent, parents, withParent, withName, pending, error, onSubmit, onClose }: {
  title: string;
  submit: string;
  name: string;
  parent: string | null;
  parents: UnitSummary[];
  withParent: boolean;
  withName: boolean;
  pending: boolean;
  error: string | undefined;
  onSubmit: (name: string, parent: string | null) => void;
  onClose: () => void;
}) {
  const t = useT();
  const [name, setName] = useState(first);
  const [parent, setParent] = useState(firstParent ?? "");
  function send(e: FormEvent) {
    e.preventDefault();
    onSubmit(name, parent || null);
  }
  return (
    <Dialog
      title={title}
      onClose={onClose}
      actions={
        <>
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
          <button type="submit" form="unit-form" className="btn btn-primary" disabled={pending || (withName && name.trim() === "")}>{submit}</button>
        </>
      }
    >
      <form id="unit-form" onSubmit={send} className="stack" style={{ gap: 12 }}>
        {withName && (
          <div className="field">
            <label htmlFor="unit-name">{t("un.name")}</label>
            <input id="unit-name" className="input" value={name} maxLength={100} onChange={(e) => setName(e.target.value)} />
          </div>
        )}
        {withParent && (
          <div className="field">
            <label htmlFor="unit-parent">{t("un.parent")}</label>
            <select id="unit-parent" className="input" value={parent} onChange={(e) => setParent(e.target.value)}>
              <option value="">{t("un.top")}</option>
              {parents.map((u) => <option key={u.id} value={u.id}>{u.name}</option>)}
            </select>
          </div>
        )}
        {error && <p role="alert" className="field-error">{error}</p>}
      </form>
    </Dialog>
  );
}

/** A new unit: top-level, or inside the one given. */
export function NewUnit({ inside, onClose }: { inside: UnitSummary | null; onClose: () => void }) {
  const t = useT();
  const create = useCreateUnit();
  return (
    <UnitForm
      title={inside ? t("un.newInsideTitle", { name: inside.name }) : t("un.newTop")}
      submit={t("un.create")}
      name=""
      parent={inside?.id ?? null}
      parents={[]}
      withParent={false}
      withName
      pending={create.pending}
      error={errorText(create.error)}
      onSubmit={async (name, parent) => { if (await create.run(name, parent)) onClose(); }}
      onClose={onClose}
    />
  );
}

export function RenameUnit({ unit, onClose }: { unit: UnitSummary; onClose: () => void }) {
  const t = useT();
  const update = useUpdateUnit(unit.id);
  return (
    <UnitForm
      title={t("un.renameTitle", { name: unit.name })}
      submit={t("cm.save")}
      name={unit.name}
      parent={unit.parent_id ?? null}
      parents={[]}
      withParent={false}
      withName
      pending={update.pending}
      error={errorText(update.error)}
      onSubmit={async (name) => { if (await update.run({ name })) onClose(); }}
      onClose={onClose}
    />
  );
}

/** Where a unit may go: anywhere but under itself or below. */
export function MoveUnit({ unit, all, onClose }: { unit: UnitSummary; all: UnitSummary[]; onClose: () => void }) {
  const t = useT();
  const update = useUpdateUnit(unit.id);
  const below = new Set<string>([unit.id]);
  // The units are flat and by name: grow the set until no unit is left whose parent is in it.
  for (let grew = true; grew; ) {
    grew = false;
    for (const u of all) if (u.parent_id && below.has(u.parent_id) && !below.has(u.id)) { below.add(u.id); grew = true; }
  }
  return (
    <UnitForm
      title={t("un.moveTitle", { name: unit.name })}
      submit={t("cm.save")}
      name=""
      parent={unit.parent_id ?? null}
      parents={all.filter((u) => !below.has(u.id))}
      withParent
      withName={false}
      pending={update.pending}
      error={errorText(update.error)}
      onSubmit={async (_name, parent) => { if (await update.run({ parentId: parent })) onClose(); }}
      onClose={onClose}
    />
  );
}
