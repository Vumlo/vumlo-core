"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Dialog } from "@/components/Dialog";
import { Icon } from "@/components/Icon";
import { ItemTile } from "@/components/ItemTile";
import { errorText } from "@/lib/api/client";
import { useMenuDismiss } from "@/lib/useMenuDismiss";
import { useContentTree, useCourseInfo, useItemMeta } from "@/lib/api/hooks/courses";
import { useCreateItem, useCreateModule, useDeleteItem, useDeleteModule, usePublishItem, usePublishModule, useReorderItems, useReorderModules, useUpdateModule } from "@/lib/api/hooks/content";
import type { ItemMeta } from "@/lib/api/types";
import type { ContentModule, ContentSubModule, ItemSummary } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { allItems, dueShort, isAssignment, typeLabel } from "@/lib/course";
import { plural } from "@/lib/format";
import { wholePoints } from "@/lib/points";
import { CourseHeader } from "./CourseHeader";
import { CourseOutline } from "./CourseOutline";
import { translate } from "@/lib/i18n/core";
import type { MessageKey } from "@/lib/i18n/core";
import type { IconName } from "@/components/Icon";
import { useT } from "@/lib/i18n";
import s from "./ContentManage.module.css";
import c from "./Course.module.css";
import { editItemHref, itemHref } from "@/lib/routes";

type Sec = ContentModule | ContentSubModule;
type Dlg =
  | { kind: "section" }
  | { kind: "rename"; sec: Sec }
  | { kind: "publish"; sec: Sec }
  | { kind: "delete-section"; sec: Sec }
  | { kind: "delete-item"; item: ItemSummary }
  | { kind: "new-item"; sec: Sec; type: "lecture" | "assignment" };

const ASSIGNMENT_TYPES = ["text", "quiz", "form", "no_submission"] as const;
const TYPE_ICON: Record<(typeof ASSIGNMENT_TYPES)[number], IconName> = { text: "text", quiz: "quiz", form: "form", no_submission: "inClass" };

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
  const info = useCourseInfo(courseId);
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
  // The new-item dialog complains about a blank title only once the person has typed or tried to create.
  const [tried, setTried] = useState(false);
  const titleBlank = text.trim() === "";
  const pointsBad = type !== "form" && points.trim() !== "" && !/^\d+(\.\d{1,2})?$/.test(points.trim());
  const pointsError = createItem.error?.errors.find((x) => x.field === "max_points")?.message;
  const [menu, setMenu] = useState<string | null>(null);
  const [drag, setDrag] = useState<{ kind: "sec" | "item"; id: string; parent: string } | null>(null);
  const [over, setOver] = useState<string | null>(null);
  useMenuDismiss(menu !== null, () => setMenu(null));
  // A publish or a reorder that the server refused would otherwise change nothing and say nothing.
  const failure = errorText(publishItem.error ?? reorderModules.error ?? reorderItems.error);

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
    setTried(false);
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
    if (titleBlank) { setTried(true); document.getElementById("item-title")?.focus(); return; }
    if (dlg.type === "assignment" && pointsBad) { document.getElementById("item-points")?.focus(); return; }
    const made = await createItem.run(dlg.sec.id, dlg.type === "lecture"
      ? { kind: "text", title: text }
      : { kind: "assignment", title: text, assignment: { type, max_points: points || "10", config: starterConfig(type, points || "10") } });
    if (made) { close(); router.push(editItemHref(courseId, made.id)); }
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
  // Items move by the menu too: dragging does not work with a keyboard or a finger.
  async function moveItem(sec: Sec, id: string, delta: -1 | 1) {
    const ids = sec.items.map((x) => x.id);
    const i = ids.indexOf(id);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= ids.length) return;
    ids.splice(i, 1); ids.splice(j, 0, id);
    setMenu(null);
    await reorderItems.run(sec.id, ids);
  }
  async function publish() {
    if (dlg?.kind !== "publish" || !choice) return;
    if (await publishModule.run(dlg.sec.id, choice === "all")) close();
  }

  // What is live is the normal state and reads as plain text; only a draft or an unpublished edit gets a pill.
  const status = (draft: boolean, editedFlag: boolean) => draft ? <span className="pill pill-dashed">{t("common.draft")}</span> : editedFlag ? <span className="pill pill-accent">{t("cm.edited")}</span> : <span className={s.live}>{t("cm.published")}</span>;
  const tick = (i: ItemSummary) => (i.status === "draft" ? "draft" : m[i.id]?.edited ? "edited" : "live");
  const publishedItems = items.filter((i) => i.status === "published").length;
  const live = Math.max(publishedItems - edited, 0);

  return (
    <div className="container">
      <div className={c.frame}>
        {tree.data && <CourseOutline courseId={courseId} tree={tree.data} meta={m} staff />}
        <div className={c.main}>
          <CourseHeader courseId={courseId} />
          <div className={s.layout}>
            <div className={s.main}>
              <p className={s.hint}>{t("cm.toolbar")}</p>
              {failure && <p role="alert" className="alert alert-danger" style={{ marginBottom: 16 }}>{failure}</p>}
              {!tree.data ? <p className="loading" role="status">{t("common.loading")}</p> : mods.length === 0 ? <p className="empty">{t("cm.empty")}</p> : (
                <div className={s.sections}>
                  {mods.map((sec, si) => (
                    <section key={sec.id} aria-label={sec.title} className={s.section} data-draft={sec.status === "draft"} data-over={drag?.kind === "sec" && over === sec.id}
                      onDragOver={(e) => { if (drag?.kind === "sec") { e.preventDefault(); setOver(sec.id); } }} onDrop={(e) => { e.preventDefault(); setOver(null); void dropSection(sec.id); }}>
                      <div className={s.sHead}>
                        <span title={t("cm.drag")} className={s.handle} draggable onDragStart={() => setDrag({ kind: "sec", id: sec.id, parent: "" })} onDragEnd={() => { setDrag(null); setOver(null); }}><Icon name="grip" size={16} strokeWidth={3} /></span>
                        <h2>{sec.title}</h2>
                        <span className={s.count}>{plural(sec.items.length, "item")}</span>
                        {sec.items.length > 0 && <span aria-hidden="true" className={s.ticks}>{sec.items.map((i) => <span key={i.id} data-s={tick(i)} />)}</span>}
                        {status(sec.status === "draft", false)}
                        {sec.status === "draft" && <button type="button" className="btn btn-outline" style={{ minHeight: 36 }} onClick={() => open({ kind: "publish", sec })}>{t("cm.publishSection")}</button>}
                        <span data-menu style={{ position: "relative" }}>
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
                      <div className={s.items}>
                        {sec.items.map((i) => {
                          const k = isAssignment(i) ? "assignment" : "lecture";
                          return (
                            <div key={i.id} className={s.item} data-kind={k} data-over={drag?.kind === "item" && over === i.id}
                              onDragOver={(e) => { if (drag?.kind === "item" && drag.parent === sec.id) { e.preventDefault(); e.stopPropagation(); setOver(i.id); } }}
                              onDrop={(e) => { e.preventDefault(); e.stopPropagation(); setOver(null); void dropItem(sec, i.id); }}>
                              <span title={t("cm.drag")} className={s.handle} draggable onDragStart={(e) => { e.stopPropagation(); setDrag({ kind: "item", id: i.id, parent: sec.id }); }} onDragEnd={() => { setDrag(null); setOver(null); }}><Icon name="grip" size={16} strokeWidth={3} /></span>
                              <ItemTile kind={k} />
                              <Link href={itemHref(courseId, i.id)} className={s.itemMain}>
                                <span className={s.itemTitle} data-draft={i.status === "draft"}>{i.title}</span>
                                <span className={s.itemMeta}>{itemMeta(i, m[i.id])}</span>
                              </Link>
                              {status(i.status === "draft", !!m[i.id]?.edited)}
                              <Link href={editItemHref(courseId, i.id)} className="btn btn-quiet" style={{ minHeight: 36 }}>{t("common.edit")}</Link>
                              <span data-menu style={{ position: "relative" }}>
                                <button type="button" className="icon-btn" aria-label={t("people.moreFor", { name: i.title })} aria-haspopup="menu" aria-expanded={menu === i.id} onClick={() => setMenu(menu === i.id ? null : i.id)}><Icon name="more" size={18} strokeWidth={3.4} /></button>
                                {menu === i.id && (
                                  <div role="menu" className="popover" style={{ top: 42, right: 0 }}>
                                    {i.status === "draft" && <button type="button" role="menuitem" className="menu-item" onClick={() => { setMenu(null); void publishItem.run(i.id); }}>{t("cm.publish")}</button>}
                                    <button type="button" role="menuitem" className="menu-item" disabled={sec.items[0]?.id === i.id} onClick={() => void moveItem(sec, i.id, -1)}>{t("cm.moveUp")}</button>
                                    <button type="button" role="menuitem" className="menu-item" disabled={sec.items[sec.items.length - 1]?.id === i.id} onClick={() => void moveItem(sec, i.id, 1)}>{t("cm.moveDown")}</button>
                                    <button type="button" role="menuitem" className="menu-item" style={{ color: "var(--danger)" }} onClick={() => open({ kind: "delete-item", item: i })}>{t("cm.delete")}</button>
                                  </div>
                                )}
                              </span>
                            </div>
                          );
                        })}
                        {sec.items.length === 0 && <p className="muted" style={{ padding: "16px 20px", fontSize: 14 }}>{t("cm.noItems")}</p>}
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
              <div className={`card card-pad ${s.ledger}`}>
                <h2 id="glance-h" className="card-title">{t("cm.glance")}</h2>
                <div aria-hidden="true" className={s.bar}>
                  <span data-s="live" style={{ flexGrow: live }} />
                  <span data-s="edited" style={{ flexGrow: edited }} />
                  <span data-s="draft" style={{ flexGrow: drafts }} />
                </div>
                <ul className={s.key} aria-label={t("cm.labels")}>
                  <li data-s="live"><span>{t("cm.arePublished", { a: plural(pubSections, "section"), b: plural(publishedItems, "item") })}</span><span className="muted">{t("cm.legendPublished")}</span></li>
                  {(mods.length - pubSections > 0 || drafts > 0) && <li data-s="draft"><span>{t("cm.areDrafts", { a: plural(mods.length - pubSections, "section"), b: plural(drafts, "item") })}</span><span className="muted">{t("cm.legendDraft")}</span></li>}
                  {edited > 0 && <li data-s="edited"><span>{t.plural("cm.edits", edited)}</span><span className="muted">{t("cm.legendEdited")}</span></li>}
                </ul>
                <div className={s.actions}>
                  <button type="button" className="btn btn-primary btn-block" onClick={() => open({ kind: "section" })}><Icon name="plus" size={16} strokeWidth={2.2} />{t("cm.addSection")}</button>
                  {firstPublished
                    ? <Link href={itemHref(courseId, firstPublished.id)} className="btn btn-outline btn-block">{t("cm.preview")}</Link>
                    : <button type="button" className="btn btn-outline btn-block" disabled>{t("cm.preview")}</button>}
                </div>
              </div>
              {info.data?.description && <p className="muted" style={{ fontSize: 13 }}>{info.data.description}</p>}
            </aside>
          </div>
        </div>
      </div>

      {(dlg?.kind === "section" || dlg?.kind === "rename") && (
        <Dialog title={dlg.kind === "section" ? t("cm.addSectionTitle") : t("cm.renameTitle")} onClose={close}
          actions={<><button type="button" className="btn" onClick={close}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" onClick={saveSection} disabled={createModule.pending || updateModule.pending}>{dlg.kind === "section" ? t("cm.addSection") : t("cm.save")}</button></>}>
          <div className="field">
            <label htmlFor="sec-title">{t("cm.sectionTitle")}</label>
            <input id="sec-title" className="input" value={text} onChange={(e) => setText(e.target.value)} aria-invalid={!!titleError(createModule.error ?? updateModule.error)} onKeyDown={(e) => { if (e.key === "Enter" && !createModule.pending && !updateModule.pending) void saveSection(); }} />
            {titleError(createModule.error ?? updateModule.error) && <p className="hint" style={{ color: "var(--danger)" }}>{titleError(createModule.error ?? updateModule.error)}</p>}
          </div>
        </Dialog>
      )}
      {dlg?.kind === "new-item" && (
        <Dialog title={dlg.type === "lecture" ? t("cm.addLecture") : t("cm.addAssignment")} description={t("cm.startsDraft", { title: dlg.sec.title })} onClose={close}
          actions={<><button type="button" className="btn" onClick={close}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" onClick={saveItem} disabled={createItem.pending}>{t("cm.createEdit")}</button></>}>
          <div className="stack" style={{ gap: 20 }}>
            <div className="field">
              <label htmlFor="item-title">{t("cm.title")}</label>
              <input id="item-title" className="input" value={text} placeholder={t(dlg.type === "lecture" ? "cm.lecturePlaceholder" : `cm.titlePlaceholder.${type}` as MessageKey)} onChange={(e) => setText(e.target.value)} onBlur={() => text.length > 0 && setTried(true)}
                onKeyDown={(e) => { if (e.key === "Enter" && !createItem.pending) void saveItem(); }} aria-invalid={(tried && titleBlank) || !!titleError(createItem.error)} aria-describedby="item-title-err" />
              <p id="item-title-err" role="alert" className="hint" style={{ color: "var(--danger)", minHeight: 18 }}>{tried && titleBlank ? t("cm.titleRequired") : titleError(createItem.error)}</p>
            </div>
            {dlg.type === "assignment" && (
              <>
                <div role="radiogroup" aria-label={t("cm.type")} className={s.types} onKeyDown={(e) => {
                  // Arrow keys move the choice, as in any radio group.
                  const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
                  if (!step) return;
                  e.preventDefault();
                  const next = ASSIGNMENT_TYPES[(ASSIGNMENT_TYPES.indexOf(type as (typeof ASSIGNMENT_TYPES)[number]) + step + ASSIGNMENT_TYPES.length) % ASSIGNMENT_TYPES.length]!;
                  setType(next);
                  e.currentTarget.querySelector<HTMLElement>(`[data-type=${next}]`)?.focus();
                }}>
                  {ASSIGNMENT_TYPES.map((v) => (
                    <button key={v} type="button" role="radio" data-type={v} aria-checked={type === v} tabIndex={type === v ? 0 : -1} className={`${s.choice} ${s.typeCard}`} onClick={() => setType(v)}>
                      <Icon name={TYPE_ICON[v]} size={20} />
                      <span><span style={{ display: "block", fontWeight: 600 }}>{typeLabel(v)}</span><span className="muted" style={{ display: "block", fontSize: 13 }}>{t(`cm.typeDesc.${v}` as MessageKey)}</span></span>
                    </button>
                  ))}
                </div>
                <p className="hint" style={{ marginTop: -8 }}>{t("cm.typeFixed")}</p>
                {type !== "form" && (
                  <div className="field">
                    <label htmlFor="item-points">{t("cm.points")}</label>
                    <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
                      <input id="item-points" className="input" inputMode="decimal" style={{ maxWidth: 110 }} value={points} onChange={(e) => setPoints(e.target.value)} aria-invalid={pointsBad || !!pointsError} aria-describedby="item-points-err" />
                      {["5", "10", "20", "50", "100"].map((p) => (
                        <button key={p} type="button" className={`btn ${points.trim() === p ? "btn-primary" : "btn-outline"}`} style={{ minHeight: 36, padding: "0 12px" }} aria-pressed={points.trim() === p} onClick={() => setPoints(p)}>{p}</button>
                      ))}
                    </div>
                    <p id="item-points-err" role="alert" className="hint" style={{ color: pointsBad || pointsError ? "var(--danger)" : undefined, minHeight: 18 }}>{pointsBad ? t("qb.pointsError") : pointsError ?? (type === "no_submission" ? t("cm.pointsHintInClass") : t("cm.pointsHint"))}</p>
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
          <div role="radiogroup" aria-label={t("cm.whatPublish")} className="stack" style={{ gap: 12 }}>
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
