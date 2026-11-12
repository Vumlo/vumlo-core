"use client";

import { useState } from "react";
import { errorText } from "@/lib/api/client";
import { useCreateItem, useUpdateItem } from "@/lib/api/hooks/content";
import { useContentTree } from "@/lib/api/hooks/courses";
import { useCreateGradebookGroup, useDeleteGradebookGroup, useGradebookStructure, usePlaceAssignments, useSaveGradebookGroup, useSaveGradebookSettings } from "@/lib/api/hooks/gradebook";
import type { GradebookColumn, GradebookGroup, GradebookLayout, GradebookStructure as Structure } from "@/lib/api/types";
import { useT } from "@/lib/i18n";
import { normalizePoints, wholePoints } from "@/lib/points";
import s from "./GradebookStructure.module.css";

const POINTS = /^\d+(\.\d{1,2})?$/;

/** The Structure mode of the gradebook (ADR-0089): how marks add up. The Grade mode only reads it. */
export function GradebookStructure({ courseId }: { courseId: string }) {
  const t = useT();
  const structure = useGradebookStructure(courseId);
  const data = structure.data;
  if (!data) return <p className="loading" role="status">{t("common.loading")}</p>;
  const custom = data.settings.layout === "custom";
  const unsorted = data.columns.filter((c) => c.group === null);
  return (
    <div className={s.page}>
      <SettingsCard courseId={courseId} data={data} />
      {data.groups.map((g) => (
        <GroupCard key={g.key} courseId={courseId} group={g} custom={custom} data={data} columns={data.columns.filter((c) => c.group === g.key)} />
      ))}
      {(unsorted.length > 0 || custom) && (
        <section className="card" aria-labelledby="gs-unsorted">
          <div className={s.head}>
            <h2 id="gs-unsorted" className={s.title}>{t("gs.notSorted")}</h2>
            <span className="muted">{unsorted.length}</span>
          </div>
          {unsorted.length > 0 && <p className={`muted ${s.hint}`}>{t("gs.notSortedHint")}</p>}
          <Items courseId={courseId} columns={unsorted} groups={custom ? data.groups : []} />
        </section>
      )}
      {custom && <Tools courseId={courseId} data={data} />}
      <AddColumn courseId={courseId} />
    </div>
  );
}

function SettingsCard({ courseId, data }: { courseId: string; data: Structure }) {
  const t = useT();
  const save = useSaveGradebookSettings(courseId);
  const [target, setTarget] = useState(wholePoints(data.settings.target));
  const [pass, setPass] = useState(data.settings.pass_mark ? wholePoints(data.settings.pass_mark) : "");
  const [layout, setLayout] = useState<GradebookLayout>(data.settings.layout);
  const { setup } = data;
  return (
    <section className="card" aria-labelledby="gs-settings">
      <div className={s.head}>
        <h2 id="gs-settings" className={s.title}>{t("gs.total")}</h2>
        <span className={s.chip} data-tone={setup.reaches_target ? "ok" : "warn"} role="status">
          {setup.reaches_target ? t("gs.possible", { possible: wholePoints(setup.possible), target: wholePoints(setup.target) }) : t("gs.reachShort", { possible: wholePoints(setup.possible), target: wholePoints(setup.target) })}
        </span>
        {setup.unsorted > 0 && <span className={s.chip} data-tone="warn">{t("gs.unsorted", { n: setup.unsorted })}</span>}
      </div>
      <form
        className={s.form}
        onSubmit={async (e) => {
          e.preventDefault();
          await save.run({ target: target.trim(), pass_mark: pass.trim() || null, layout });
        }}
      >
        <div className="field">
          <label htmlFor="gs-target">{t("gs.target")}</label>
          <input id="gs-target" className="input" inputMode="decimal" value={target} onChange={(e) => setTarget(e.target.value)} />
        </div>
        <div className="field">
          <label htmlFor="gs-pass">{t("gs.passMark")}</label>
          <input id="gs-pass" className="input" inputMode="decimal" value={pass} onChange={(e) => setPass(e.target.value)} aria-describedby="gs-pass-hint" />
          <p id="gs-pass-hint" className="hint">{t("gs.passMarkHint")}</p>
        </div>
        <fieldset className={s.layouts}>
          <legend>{t("gs.layout")}</legend>
          {(["course", "custom"] as const).map((l) => (
            <label key={l} className="check">
              <input type="radio" name="gs-layout" checked={layout === l} onChange={() => setLayout(l)} />
              <span>{t(l === "course" ? "gs.layoutCourse" : "gs.layoutCustom")}<span className="muted"> · {t(l === "course" ? "gs.layoutCourseHint" : "gs.layoutCustomHint")}</span></span>
            </label>
          ))}
        </fieldset>
        <div>
          <button type="submit" className="btn btn-primary" disabled={save.pending}>{t("gs.saveSettings")}</button>
        </div>
        {save.error && <p role="alert" className="alert alert-danger">{errorText(save.error)}</p>}
      </form>
    </section>
  );
}

function ruleChips(t: ReturnType<typeof useT>, g: GradebookGroup): string[] {
  return [
    g.cap ? t("gs.ruleCap", { n: wholePoints(g.cap) }) : "",
    g.best_k ? t("gs.ruleBest", { k: g.best_k }) : "",
    !g.counts ? t("gs.rulePractice") : "",
    g.bonus ? t("gs.ruleBonus") : "",
  ].filter(Boolean);
}

function GroupCard({ courseId, group, custom, data, columns }: { courseId: string; group: GradebookGroup; custom: boolean; data: Structure; columns: GradebookColumn[] }) {
  const t = useT();
  const save = useSaveGradebookGroup(courseId);
  const remove = useDeleteGradebookGroup(courseId);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(group.title);
  const [cap, setCap] = useState(group.cap ? wholePoints(group.cap) : "");
  const [bestK, setBestK] = useState(group.best_k ? String(group.best_k) : "");
  const [counts, setCounts] = useState(group.counts);
  const [bonus, setBonus] = useState(group.bonus);
  const chips = ruleChips(t, group);
  const err = save.error ?? remove.error;
  return (
    <section className="card" aria-labelledby={`gs-g-${group.key}`}>
      <div className={s.head}>
        <h2 id={`gs-g-${group.key}`} className={s.title}>{group.title}</h2>
        {chips.map((c) => <span key={c} className={s.chip}>{c}</span>)}
        <span style={{ flex: "1 1 auto" }} />
        <button type="button" className="btn" aria-expanded={editing} onClick={() => setEditing(!editing)}>{t("gs.editRules")}</button>
        {custom && <button type="button" className="btn" disabled={remove.pending} onClick={() => remove.run(group.key)}>{t("gs.deleteGroup")}</button>}
      </div>
      {editing && (
        <form
          className={s.form}
          onSubmit={async (e) => {
            e.preventDefault();
            const k = bestK.trim();
            const done = await save.run(group.key, {
              ...(custom ? { title: title.trim() } : {}),
              cap: cap.trim() || null,
              best_k: k ? Number(k) : null,
              counts,
              bonus: counts && bonus,
              assignments: [],
            });
            if (done) setEditing(false);
          }}
        >
          {custom && (
            <div className="field">
              <label htmlFor={`gs-t-${group.key}`}>{t("gs.groupTitle")}</label>
              <input id={`gs-t-${group.key}`} className="input" value={title} onChange={(e) => setTitle(e.target.value)} />
            </div>
          )}
          <div className="field">
            <label htmlFor={`gs-c-${group.key}`}>{t("gs.cap")}</label>
            <input id={`gs-c-${group.key}`} className="input" inputMode="decimal" value={cap} onChange={(e) => setCap(e.target.value)} aria-describedby={`gs-ch-${group.key}`} />
            <p id={`gs-ch-${group.key}`} className="hint">{t("gs.capHint")}</p>
          </div>
          <div className="field">
            <label htmlFor={`gs-k-${group.key}`}>{t("gs.bestK")}</label>
            <input id={`gs-k-${group.key}`} className="input" inputMode="numeric" value={bestK} onChange={(e) => setBestK(e.target.value)} aria-describedby={`gs-kh-${group.key}`} />
            <p id={`gs-kh-${group.key}`} className="hint">{t("gs.bestKHint")}</p>
          </div>
          <label className="check"><input type="checkbox" checked={counts} onChange={(e) => setCounts(e.target.checked)} />{t("gs.counts")}</label>
          <label className="check"><input type="checkbox" checked={counts && bonus} disabled={!counts} onChange={(e) => setBonus(e.target.checked)} />{t("gs.bonus")}</label>
          <div><button type="submit" className="btn btn-primary" disabled={save.pending}>{t("cm.save")}</button></div>
        </form>
      )}
      {err && <p role="alert" className="alert alert-danger">{errorText(err)}</p>}
      {columns.length === 0 ? <p className={`muted ${s.hint}`}>{t("gs.emptyGroup")}</p> : <Items courseId={courseId} columns={columns} groups={custom ? data.groups : []} />}
    </section>
  );
}

/** The assignments of a group. `groups` is empty when they cannot be moved (the layout follows the course). */
function Items({ courseId, columns, groups }: { courseId: string; columns: GradebookColumn[]; groups: GradebookGroup[] }) {
  const t = useT();
  const place = usePlaceAssignments(courseId);
  return (
    <>
      <ul className={s.items}>
        {columns.map((c) => (
          <li key={c.assignment_id} className={s.item}>
            <span className={s.itemTitle}>{c.title}{c.is_bonus && <span className={s.chip}>{t("gs.bonusItem")}</span>}</span>
            <PointsField courseId={courseId} column={c} />
            {groups.length > 0 && (
              <select aria-label={t("gs.moveTo", { title: c.title })} className="select" style={{ width: "auto", minHeight: 36 }} value={c.group ?? ""} onChange={(e) => place.run([c.assignment_id], e.target.value || null)}>
                <option value="">{t("gs.notSorted")}</option>
                {groups.map((g) => <option key={g.key} value={g.key}>{g.title}</option>)}
              </select>
            )}
          </li>
        ))}
      </ul>
      {place.error && <p role="alert" className="alert alert-danger">{errorText(place.error)}</p>}
    </>
  );
}

/** Points of one assignment, saved when the field loses focus. */
function PointsField({ courseId, column }: { courseId: string; column: GradebookColumn }) {
  const t = useT();
  const update = useUpdateItem(courseId);
  const [value, setValue] = useState(wholePoints(column.max_points));
  const bad = !POINTS.test(value.trim());
  return (
    <span className={s.points}>
      <input
        aria-label={t("gs.points", { title: column.title })}
        aria-invalid={bad}
        className="input"
        inputMode="decimal"
        style={{ width: 84, minHeight: 36 }}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onBlur={async () => {
          if (bad) return setValue(wholePoints(column.max_points));
          const next = normalizePoints(value.trim());
          if (next === column.max_points) return;
          if (!(await update.run(column.assignment_id, { max_points: next }))) setValue(wholePoints(column.max_points));
        }}
      />
      {update.error && <span role="alert" className={s.error}>{errorText(update.error)}</span>}
    </span>
  );
}

/** Suggestions and tidying of the teacher's own layout. */
function Tools({ courseId, data }: { courseId: string; data: Structure }) {
  const t = useT();
  const create = useCreateGradebookGroup(courseId);
  const remove = useDeleteGradebookGroup(courseId);
  const [name, setName] = useState("");
  const [shown, setShown] = useState(false);
  const titleOf = new Map(data.columns.map((c) => [c.assignment_id, c.title]));
  return (
    <section className="card" aria-label={t("gs.newGroup")}>
      <form
        className={s.row}
        onSubmit={async (e) => {
          e.preventDefault();
          if (name.trim() && (await create.run({ title: name.trim(), counts: true, bonus: false, assignments: [] }))) setName("");
        }}
      >
        <input aria-label={t("gs.groupTitle")} className="input" style={{ maxWidth: 260 }} value={name} onChange={(e) => setName(e.target.value)} />
        <button type="submit" className="btn" disabled={create.pending || !name.trim()}>{t("gs.newGroup")}</button>
        <button type="button" className="btn" aria-expanded={shown} onClick={() => setShown(!shown)}>{t("gs.suggest")}</button>
        {data.setup.empty_groups.length > 0 && (
          <button type="button" className="btn" disabled={remove.pending} onClick={async () => { for (const key of data.setup.empty_groups) await remove.run(key); }}>{t("gs.tidy")}</button>
        )}
      </form>
      {shown && (data.suggestions.length === 0 ? <p className={`muted ${s.hint}`}>{t("gs.suggestNone")}</p> : (
        <ul className={s.items}>
          {data.suggestions.map((sg) => (
            <li key={sg.title} className={s.item}>
              <span className={s.itemTitle}>{sg.assignments.map((id) => titleOf.get(id)).filter(Boolean).join(", ")}</span>
              <button type="button" className="btn" disabled={create.pending} onClick={() => create.run({ title: sg.title, counts: true, bonus: false, assignments: sg.assignments })}>{t("gs.suggestMake", { title: sg.title, n: sg.assignments.length })}</button>
            </li>
          ))}
        </ul>
      ))}
      {(create.error ?? remove.error) && <p role="alert" className="alert alert-danger">{errorText((create.error ?? remove.error)!)}</p>}
    </section>
  );
}

/** A column filled in by hand: an assignment with nothing to submit (ADR-0009). */
function AddColumn({ courseId }: { courseId: string }) {
  const t = useT();
  const tree = useContentTree(courseId);
  const create = useCreateItem(courseId);
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [points, setPoints] = useState("10");
  const [moduleId, setModuleId] = useState("");
  const modules = tree.data?.modules ?? [];
  const target = moduleId || modules[0]?.id;
  const ok = !!title.trim() && POINTS.test(points.trim()) && !!target;
  return (
    <section className="card" aria-label={t("gs.addColumn")}>
      <div className={s.row}>
        <button type="button" className="btn" aria-expanded={open} onClick={() => setOpen(!open)}>+ {t("gs.addColumn")}</button>
        {open && <span className="muted">{t("gs.colHint")}</span>}
      </div>
      {open && (
        <form
          className={s.form}
          onSubmit={async (e) => {
            e.preventDefault();
            if (!ok || !target) return;
            if (await create.run(target, { kind: "assignment", title: title.trim(), assignment: { type: "no_submission", max_points: normalizePoints(points.trim()) } })) setTitle("");
          }}
        >
          <div className="field">
            <label htmlFor="gs-col-title">{t("gs.colTitle")}</label>
            <input id="gs-col-title" className="input" value={title} onChange={(e) => setTitle(e.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="gs-col-points">{t("gs.colPoints")}</label>
            <input id="gs-col-points" className="input" inputMode="decimal" value={points} onChange={(e) => setPoints(e.target.value)} />
          </div>
          <div className="field">
            <label htmlFor="gs-col-module">{t("gs.colModule")}</label>
            <select id="gs-col-module" className="select" value={target ?? ""} onChange={(e) => setModuleId(e.target.value)}>
              {modules.map((m) => <option key={m.id} value={m.id}>{m.title}</option>)}
            </select>
          </div>
          <div><button type="submit" className="btn btn-primary" disabled={!ok || create.pending}>{t("gs.colAdd")}</button></div>
          {create.error && <p role="alert" className="alert alert-danger">{errorText(create.error)}</p>}
        </form>
      )}
    </section>
  );
}
