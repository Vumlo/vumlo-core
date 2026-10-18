"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { ItemTile } from "@/components/ItemTile";
import { useContentTree, useItemMeta } from "@/lib/api/hooks/courses";
import { useCreateItem, useCreateModule, useDeleteItem, useDeleteModule, usePublishItem, usePublishModule, useReorderItems, useReorderModules, useUpdateModule } from "@/lib/api/hooks/content";
import type { ItemMeta } from "@/lib/api/types";
import type { ContentModule, ContentSubModule, ItemSummary } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { allItems, dueShort, isAssignment, typeLabel } from "@/lib/course";
import { plural } from "@/lib/format";
import { wholePoints } from "@/lib/points";
import { CourseHeader } from "./CourseHeader";
import { translate } from "@/lib/i18n/core";
import { useT } from "@/lib/i18n";
import s from "./ContentManage.module.css";

type Sec = ContentModule | ContentSubModule;
type Dlg =
  | { kind: "section" }
  | { kind: "rename"; sec: Sec }
  | { kind: "publish"; sec: Sec }
  | { kind: "delete-section"; sec: Sec }
  | { kind: "delete-item"; item: ItemSummary }
  | { kind: "new-item"; sec: Sec; type: "lecture" | "assignment" };

const ASSIGNMENT_TYPES = ["text", "quiz", "form", "no_submission"] as const;

/** The server wants a quiz to have a question and a form a field from the start (ADR-0050, ADR-0051). */
function starterConfig(type: string, points: string): Record<string, unknown> | undefined {
  if (type === "quiz") {
    return { questions: [{ id: "q1", type: "single", prompt: "", points, options: [{ id: "o1", text: "" }, { id: "o2", text: "" }], correct: ["o1"] }], shuffle_questions: false, show_correct: true };
  }
  if (type === "form") return { note: "", fields: [{ id: "f1", type: "short", label: "Field 1", help: "", required: false, options: [], rules: {}, has_answers: false }] };
  return undefined;
}

function itemMeta(i: ItemSummary, m: ItemMeta | undefined): string {
  if (!isAssignment(i)) return `${translate("row.lecture")}${m?.read_minutes ? `, ${translate("row.readMinutes", { n: m.read_minutes })}` : ""}`;
  const a = i.assignment!;
  const when = dueShort(a.due_at ?? null, m?.class_on, now());
  const pts = a.max_points === "0.00" ? translate("row.notGraded") : translate("row.points", { points: wholePoints(a.max_points) });
  return `${typeLabel(a.type)}, ${pts}${when === translate("due.none") ? "" : `, ${when.charAt(0).toLowerCase()}${when.slice(1)}`}`;
}

export function ContentManage({ courseId }: { courseId: string }) {
  const t = useT();
  const router = useRouter();
  const tree = useContentTree(courseId);
  const meta = useItemMeta(courseId);
  const createModule = useCreateModule(courseId);
  const updateModule = useUpdateModule(courseId);
  const deleteModule = useDeleteModule(courseId);
  const publishModule = usePublishModule(courseId);
  const createItem = useCreateItem(courseId);
  const publishItem = usePublishItem(courseId);
  const deleteItem = useDeleteItem(courseId);
  const reorderModules = useReorderModules(courseId);
  const reorderItems = useReorderItems(courseId);
  const [dlg, setDlg] = useState<Dlg | null>(null);
  const [text, setText] = useState("");
  const [type, setType] = useState<string>("text");
  const [points, setPoints] = useState("10");
  const [choice, setChoice] = useState<"only" | "all" | null>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [drag, setDrag] = useState<{ kind: "sec" | "item"; id: string; parent: string } | null>(null);
  const [over, setOver] = useState<string | null>(null);

  const m = meta.data?.items ?? {};
  const mods = tree.data?.modules ?? [];
  const items = tree.data ? allItems(tree.data) : [];
  const pubSections = mods.filter((x) => x.status === "published").length;
  const drafts = items.filter((i) => i.status === "draft").length;
  const edited = items.filter((i) => m[i.id]?.edited).length;
  const firstPublished = items.find((i) => i.status === "published");

  function open(d: Dlg) {
    setText(d.kind === "rename" ? d.sec.title : "");
    setType("text");
    setPoints("10");
    setChoice(null);
    setMenu(null);
    createModule.reset(); createItem.reset(); updateModule.reset(); publishModule.reset(); deleteModule.reset(); deleteItem.reset();
    setDlg(d);
  }
  const close = () => setDlg(null);
  const titleError = (e?: { errors: { field: string; message: string }[] }) => e?.errors.find((x) => x.field === "title")?.message;

  async function saveSection() {
    if (dlg?.kind === "section" && (await createModule.run(text))) close();
    if (dlg?.kind === "rename" && (await updateModule.run(dlg.sec.id, { title: text }))) close();
  }
  async function saveItem() {
    if (dlg?.kind !== "new-item") return;
    const made = await createItem.run(dlg.sec.id, dlg.type === "lecture"
      ? { kind: "text", title: text }
      : { kind: "assignment", title: text, assignment: { type, max_points: points || "10", config: starterConfig(type, points || "10") } });
    if (made) { close(); router.push(`/courses/${courseId}/items/${made.id}/edit`); }
  }

  function reorder<T extends { id: string }>(list: T[], from: string, to: string): string[] | null {
    const ids = list.map((x) => x.id);
    const a = ids.indexOf(from);
    const b = ids.indexOf(to);
    if (a < 0 || b < 0 || a === b) return null;
    ids.splice(a, 1);
    ids.splice(b, 0, from);
    return ids;
  }
  async function dropSection(to: string) {
    if (drag?.kind !== "sec") return;
    const ids = reorder(mods, drag.id, to);
    if (ids) await reorderModules.run(ids);
  }
  async function dropItem(sec: Sec, to: string) {
    if (drag?.kind !== "item" || drag.parent !== sec.id) return;
    const ids = reorder(sec.items, drag.id, to);
    if (ids) await reorderItems.run(sec.id, ids);
  }
  async function moveSection(id: string, delta: -1 | 1) {
    const ids = mods.map((x) => x.id);
    const i = ids.indexOf(id);
    const j = i + delta;
    if (j < 0 || j >= ids.length) return;
    ids.splice(i, 1); ids.splice(j, 0, id);
    setMenu(null);
    await reorderModules.run(ids);
  }
  async function publish() {
    if (dlg?.kind !== "publish" || !choice) return;
    if (await publishModule.run(dlg.sec.id, choice === "all")) close();
  }

  const status = (draft: boolean, editedFlag: boolean) => draft ? <span className="pill pill-dashed">{t("common.draft")}</span> : editedFlag ? <span className="pill pill-accent">{t("cm.edited")}</span> : <span className="pill pill-ok">{t("cm.published")}</span>;

  return (
    <div className="container">
      <CourseHeader courseId={courseId} />
      <div className={s.layout}>
        <div className={s.main}>
          <div className={s.toolbar}>
            <p>{t("cm.toolbar")}</p>
            {firstPublished
              ? <Link href={`/courses/${courseId}/items/${firstPublished.id}`} className="btn btn-outline">{t("cm.preview")}</Link>
              : <button type="button" className="btn btn-outline" disabled>{t("cm.preview")}</button>}
            <button type="button" className="btn btn-primary" onClick={() => open({ kind: "section" })}><Icon name="plus" size={16} strokeWidth={2.2} />{t("cm.addSection")}</button>
          </div>
          {!tree.data ? <p className="loading" role="status">{t("common.loading")}</p> : mods.length === 0 ? <p className="empty">{t("cm.empty")}</p> : (
            <div className={s.sections}>
              {mods.map((sec, si) => (
                <section key={sec.id} aria-label={sec.title} className={s.section} data-draft={sec.status === "draft"} data-over={drag?.kind === "sec" && over === sec.id}
                  onDragOver={(e) => { if (drag?.kind === "sec") { e.preventDefault(); setOver(sec.id); } }} onDrop={(e) => { e.preventDefault(); setOver(null); void dropSection(sec.id); }}>
                  <div className={s.sHead}>
                    <span title={t("cm.drag")} className={s.handle} draggable onDragStart={() => setDrag({ kind: "sec", id: sec.id, parent: "" })} onDragEnd={() => { setDrag(null); setOver(null); }}><Icon name="grip" size={16} strokeWidth={3} /></span>
                    <h2>{sec.title}</h2>
                    <span className="muted" style={{ fontSize: 13 }}>{plural(sec.items.length, "item")}</span>
                    {status(sec.status === "draft", false)}
                    {sec.status === "draft" && <button type="button" className="btn btn-outline" style={{ minHeight: 36 }} onClick={() => open({ kind: "publish", sec })}>{t("cm.publishSection")}</button>}
                    <span style={{ position: "relative" }}>
                      <button type="button" className="icon-btn" aria-label={t("people.moreFor", { name: sec.title })} aria-haspopup="menu" aria-expanded={menu === sec.id} onClick={() => setMenu(menu === sec.id ? null : sec.id)}><Icon name="more" size={18} strokeWidth={3.4} /></button>
                      {menu === sec.id && (
                        <div role="menu" className="popover" style={{ top: 42, right: 0 }}>
                          <button type="button" role="menuitem" className="menu-item" onClick={() => open({ kind: "rename", sec })}>{t("cm.rename")}</button>
                          <button type="button" role="menuitem" className="menu-item" disabled={si === 0} onClick={() => void moveSection(sec.id, -1)}>{t("cm.moveUp")}</button>
                          <button type="button" role="menuitem" className="menu-item" disabled={si === mods.length - 1} onClick={() => void moveSection(sec.id, 1)}>{t("cm.moveDown")}</button>
                          <button type="button" role="menuitem" className="menu-item" style={{ color: "var(--danger)" }} onClick={() => open({ kind: "delete-section", sec })}>{t("cm.deleteSection")}</button>
                        </div>
                      )}
                    </span>
                  </div>
                  <div>
                    {sec.items.map((i) => {
                      const k = isAssignment(i) ? "assignment" : "lecture";
                      return (
                        <div key={i.id} className={s.item} data-kind={k} data-over={drag?.kind === "item" && over === i.id}
                          onDragOver={(e) => { if (drag?.kind === "item" && drag.parent === sec.id) { e.preventDefault(); e.stopPropagation(); setOver(i.id); } }}
                          onDrop={(e) => { e.preventDefault(); e.stopPropagation(); setOver(null); void dropItem(sec, i.id); }}>
                          <span title={t("cm.drag")} className={s.handle} draggable onDragStart={(e) => { e.stopPropagation(); setDrag({ kind: "item", id: i.id, parent: sec.id }); }} onDragEnd={() => { setDrag(null); setOver(null); }}><Icon name="grip" size={16} strokeWidth={3} /></span>
                          <ItemTile kind={k} />
                          <Link href={`/courses/${courseId}/items/${i.id}`} className={s.itemMain}>
                            <span className={s.itemTitle} data-draft={i.status === "draft"}>{i.title}</span>
                            <span className={s.itemMeta}>{itemMeta(i, m[i.id])}</span>
                          </Link>
                          {status(i.status === "draft", !!m[i.id]?.edited)}
                          <Link href={`/courses/${courseId}/items/${i.id}/edit`} className="btn btn-quiet" style={{ minHeight: 36 }}>{t("common.edit")}</Link>
                          <span style={{ position: "relative" }}>
                            <button type="button" className="icon-btn" aria-label={t("people.moreFor", { name: i.title })} aria-haspopup="menu" aria-expanded={menu === i.id} onClick={() => setMenu(menu === i.id ? null : i.id)}><Icon name="more" size={18} strokeWidth={3.4} /></button>
                            {menu === i.id && (
                              <div role="menu" className="popover" style={{ top: 42, right: 0 }}>
                                {i.status === "draft" && <button type="button" role="menuitem" className="menu-item" onClick={() => { setMenu(null); void publishItem.run(i.id); }}>{t("cm.publish")}</button>}
                                <button type="button" role="menuitem" className="menu-item" style={{ color: "var(--danger)" }} onClick={() => open({ kind: "delete-item", item: i })}>{t("cm.delete")}</button>
                              </div>
                            )}
                          </span>
                        </div>
                      );
                    })}
                    {sec.items.length === 0 && <p className="muted" style={{ padding: "14px 18px", fontSize: 14 }}>{t("cm.noItems")}</p>}
                  </div>
                  <div className={s.add}>
                    <button type="button" className="btn" style={{ minHeight: 36 }} onClick={() => open({ kind: "new-item", sec, type: "lecture" })}><Icon name="plus" size={15} strokeWidth={2.2} />{t("row.lecture")}</button>
                    <button type="button" className="btn" style={{ minHeight: 36 }} onClick={() => open({ kind: "new-item", sec, type: "assignment" })}><Icon name="plus" size={15} strokeWidth={2.2} />{t("cm.assignment")}</button>
                  </div>
                </section>
              ))}
            </div>
          )}
        </div>
        <aside aria-labelledby="glance-h" className={s.aside}>
          <div className="card card-pad">
            <h2 id="glance-h" className="card-title">{t("cm.glance")}</h2>
            <ul className={s.glance}>
              <li>{t("cm.arePublished", { a: plural(pubSections, "section"), b: plural(items.filter((i) => i.status === "published").length, "item") })}</li>
              {(mods.length - pubSections > 0 || drafts > 0) && <li>{t("cm.areDrafts", { a: plural(mods.length - pubSections, "section"), b: plural(drafts, "item") })}</li>}
              {edited > 0 && <li>{t.plural("cm.edits", edited)}</li>}
            </ul>
          </div>
          <div className="card card-pad">
            <h2 className="card-title">{t("cm.labels")}</h2>
            <div className={s.legend}>
              <span className="line"><span className="pill pill-ok">{t("cm.published")}</span> {t("cm.legendPublished")}</span>
              <span className="line"><span className="pill pill-dashed">{t("common.draft")}</span> {t("cm.legendDraft")}</span>
              <span className="line"><span className="pill pill-accent">{t("cm.edited")}</span> {t("cm.legendEdited")}</span>
            </div>
          </div>
        </aside>
      </div>

      {(dlg?.kind === "section" || dlg?.kind === "rename") && (
        <Dialog title={dlg.kind === "section" ? t("cm.addSectionTitle") : t("cm.renameTitle")} onClose={close}
          actions={<><button type="button" className="btn" onClick={close}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" onClick={saveSection} disabled={createModule.pending || updateModule.pending}>{dlg.kind === "section" ? t("cm.addSection") : t("cm.save")}</button></>}>
          <div className="field">
            <label htmlFor="sec-title">{t("cm.sectionTitle")}</label>
            <input id="sec-title" className="input" value={text} onChange={(e) => setText(e.target.value)} aria-invalid={!!titleError(createModule.error ?? updateModule.error)} onKeyDown={(e) => { if (e.key === "Enter") void saveSection(); }} />
            {titleError(createModule.error ?? updateModule.error) && <p className="hint" style={{ color: "var(--danger)" }}>{titleError(createModule.error ?? updateModule.error)}</p>}
          </div>
        </Dialog>
      )}
      {dlg?.kind === "new-item" && (
        <Dialog title={dlg.type === "lecture" ? t("cm.addLecture") : t("cm.addAssignment")} description={t("cm.startsDraft", { title: dlg.sec.title })} onClose={close}
          actions={<><button type="button" className="btn" onClick={close}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" onClick={saveItem} disabled={createItem.pending}>{t("cm.createEdit")}</button></>}>
          <div className="stack" style={{ gap: 14 }}>
            <div className="field">
              <label htmlFor="item-title">{t("cm.title")}</label>
              <input id="item-title" className="input" value={text} onChange={(e) => setText(e.target.value)} aria-invalid={!!titleError(createItem.error)} />
              {titleError(createItem.error) && <p className="hint" style={{ color: "var(--danger)" }}>{titleError(createItem.error)}</p>}
            </div>
            {dlg.type === "assignment" && (
              <>
                <div className="field">
                  <label htmlFor="item-type">{t("cm.type")}</label>
                  <select id="item-type" className="select" value={type} onChange={(e) => setType(e.target.value)}>
                    {ASSIGNMENT_TYPES.map((v) => <option key={v} value={v}>{typeLabel(v)}</option>)}
                  </select>
                  <p className="hint">{t("cm.typeFixed")}</p>
                </div>
                {type !== "form" && (
                  <div className="field">
                    <label htmlFor="item-points">{t("cm.points")}</label>
                    <input id="item-points" className="input" inputMode="decimal" style={{ maxWidth: 160 }} value={points} onChange={(e) => setPoints(e.target.value)} aria-invalid={!!createItem.error?.errors.find((x) => x.field === "max_points")} />
                    {createItem.error?.errors.find((x) => x.field === "max_points") && <p className="hint" style={{ color: "var(--danger)" }}>{createItem.error.errors.find((x) => x.field === "max_points")!.message}</p>}
                  </div>
                )}
              </>
            )}
          </div>
        </Dialog>
      )}
      {dlg?.kind === "publish" && (
        <Dialog title={t("cm.publishQ", { title: dlg.sec.title })} description={t("cm.publishLead", { n: plural(dlg.sec.items.filter((i) => i.status === "draft").length, "draft item") })} onClose={close}
          actions={<><button type="button" className="btn" onClick={close}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" disabled={!choice || publishModule.pending} onClick={publish}>{t("cm.publish")}</button></>}>
          <div role="radiogroup" aria-label={t("cm.whatPublish")} className="stack" style={{ gap: 10 }}>
            {([
              ["only", t("cm.onlySection"), t("cm.onlyDesc", { n: plural(dlg.sec.items.length, "item") })],
              ["all", t("cm.allSection"), t("cm.allDesc", { n: plural(dlg.sec.items.length, "item") })],
            ] as const).map(([id, title, desc]) => (
              <button key={id} type="button" role="radio" aria-checked={choice === id} className={s.choice} onClick={() => setChoice(id)}>
                <span className={s.radio} aria-hidden="true"><span className={s.dot} /></span>
                <span><span style={{ display: "block", fontWeight: 600 }}>{title}</span><span className="muted" style={{ display: "block", fontSize: 14 }}>{desc}</span></span>
              </button>
            ))}
          </div>
          <p className="muted" style={{ marginTop: 12, fontSize: 13, minHeight: 20 }}>{choice ? "" : t("cm.pickOne")}</p>
          {publishModule.error && <p role="alert" className="alert alert-danger">{publishModule.error.detail ?? publishModule.error.title}</p>}
        </Dialog>
      )}
      {dlg?.kind === "delete-section" && (
        <ConfirmDialog title={t("cm.deleteQ", { title: dlg.sec.title })} confirm={t("cm.deleteSection")} danger pending={deleteModule.pending} error={deleteModule.error?.detail} onClose={close} onConfirm={async () => { if (await deleteModule.run(dlg.sec.id)) close(); }}>
          {t("cm.deleteSectionLead", { n: plural(dlg.sec.items.length, "item") })}
        </ConfirmDialog>
      )}
      {dlg?.kind === "delete-item" && (
        <ConfirmDialog title={t("cm.deleteQ", { title: dlg.item.title })} confirm={t("cm.delete")} danger pending={deleteItem.pending} error={deleteItem.error?.detail} onClose={close} onConfirm={async () => { if (await deleteItem.run(dlg.item.id)) close(); }}>
          {t("cm.deleteItemLead")}
        </ConfirmDialog>
      )}
    </div>
  );
}
