"use client";

import Link from "next/link";
import { useMemo, useRef, useState } from "react";
import { BlockEditor, type EditorHandle, type EditorState } from "@/components/doc/BlockEditor";
import { plainText } from "@/components/doc/BlockRenderer";
import { blockMenu, type InsertKind } from "@/components/doc/model";
import { Icon } from "@/components/Icon";
import { useEditorProfiles, usePublishItem, usePublishRevision, useItemRevision, useUpdateItem } from "@/lib/api/hooks/content";
import { useCourseInfo } from "@/lib/api/hooks/courses";
import { useAutosave } from "@/lib/autosave";
import { dayLong, timeOf } from "@/lib/format";
import { bodyOf, type Document, type Feature, type Item } from "@/lib/api/types";
import { AuthorBar } from "./AuthorBar";
import { useT } from "@/lib/i18n";
import { locale } from "@/lib/i18n/core";
import type { MessageKey } from "@/lib/i18n/core";
import s from "./Author.module.css";

const FULL: Feature[] = ["headings", "text_styles", "links", "lists", "quote", "divider", "table", "code", "math", "mermaid", "video"];
const TURN: [InsertKind, string, MessageKey][] = [["paragraph", "T", "le.turn.text"], ["h1", "H1", "le.turn.h1"], ["h2", "H2", "le.turn.h2"], ["h3", "H3", "le.turn.h3"], ["quote", "“", "le.turn.quote"], ["bulleted", "•", "le.turn.bulleted"], ["numbered", "1.", "le.turn.numbered"], ["check", "✓", "le.turn.check"]];

export function LectureEditor({ item, courseId }: { item: Item; courseId: string }) {
  const t = useT();
  const profiles = useEditorProfiles();
  const info = useCourseInfo(courseId);
  const revision = useItemRevision(item.id);
  const update = useUpdateItem(courseId);
  const publishItem = usePublishItem(courseId);
  const publishRevision = usePublishRevision(item.id, courseId);
  const handle = useRef<EditorHandle>(null);
  const [initial] = useState(() => bodyOf(item));
  const [doc, setDoc] = useState<Document>(initial);
  const [title, setTitle] = useState(item.title);
  const [tab, setTab] = useState<"insert" | "style" | "page">("insert");
  const [find, setFind] = useState("");
  const [state, setState] = useState<EditorState>({ activeId: null, activeKind: null, formats: { bold: false, italic: false, strike: false, code: false } });
  const features = profiles.data?.profiles.find((p) => p.name === "full")?.features ?? FULL;
  const limit = profiles.data?.profiles.find((p) => p.name === "full")?.limits.max_document_bytes ?? 1_048_576;

  const draft = useMemo(() => ({ doc, title }), [doc, title]);
  const auto = useAutosave(draft, (d) => update.run(item.id, { title: d.title.trim() || item.title, body: d.doc }));
  const words = useMemo(() => plainText(doc).split(/\s+/).filter(Boolean).length, [doc]);
  const minutes = Math.max(1, Math.round(words / 200));
  const bytes = useMemo(() => JSON.stringify(doc).length, [doc]);
  const rev = revision.data;
  const isDraft = item.status === "draft";
  const changed = !isDraft && !!rev?.has_unpublished_changes;
  const chip = isDraft ? { text: t("le.draftChip"), tone: "draft" as const } : changed ? { text: t("le.unpublishedChip"), tone: "accent" as const } : { text: t("cm.published"), tone: "ok" as const };

  async function publish() {
    await auto.flush();
    if (isDraft) await publishItem.run(item.id);
    else await publishRevision.run();
  }
  const lastPublished = rev?.last_published_at;
  const canPublish = isDraft || changed;

  return (
    <div className="container" style={{ paddingTop: 20 }}>
      <AuthorBar courseId={courseId} itemId={item.id} title={title} chip={chip} saved={auto.savedAt} pending={auto.pending} failed={auto.failed}>
        <Link href={`/courses/${courseId}/items/${item.id}`} className="btn">{t("cm.preview")}</Link>
        <button type="button" className="btn btn-primary" disabled={!canPublish || publishItem.pending || publishRevision.pending} onClick={publish}>{isDraft ? t("cm.publish") : t("le.publishChanges")}</button>
      </AuthorBar>
      {!isDraft && chip.tone === "accent" && lastPublished && <p className="muted" style={{ margin: "-4px 2px 16px", fontSize: 14 }}>{t("le.seeOld", { day: dayLong(lastPublished) })}</p>}
      <div className={s.shell}>
        <div className={s.canvas}>
          <div className={s.sheet}>
            <div className="row" style={{ gap: 8, marginBottom: 10 }}>
              <span aria-hidden="true" style={{ display: "flex", alignItems: "center", justifyContent: "center", width: 26, height: 26, borderRadius: 7, background: "var(--lec-tile)", color: "var(--lec-fg)" }}><Icon name="lecture" size={15} strokeWidth={2} /></span>
              <span style={{ fontSize: 14, fontWeight: 600, color: "var(--lec-fg)" }}>{t("row.lecture")}</span>
              <span className="muted" style={{ fontSize: 14 }}>{t("le.about", { n: minutes })}</span>
            </div>
            <label htmlFor="title" className="sr-only">{t("cm.title")}</label>
            <input id="title" className={s.titleInput} value={title} onChange={(e) => setTitle(e.target.value)} aria-invalid={!title.trim()} />
            {!title.trim() && <p className="hint" style={{ color: "var(--danger)", marginTop: 6 }}>{t("le.needsTitle")}</p>}
            <div style={{ marginTop: 16, paddingLeft: 34 }}>
              <BlockEditor handle={handle} value={initial} onChange={setDoc} features={features} label={t("le.content")} mode="float" minHeight={420} onState={setState} />
            </div>
          </div>
          <p className="muted" style={{ margin: "12px 2px 0", fontSize: 13 }}>{t("le.images")}</p>
        </div>

        <aside aria-label={t("le.panel")} className={`card ${s.panel}`}>
          <div role="tablist" aria-label={t("le.panelSections")} className={s.panelTabs}>
            {(["insert", "style", "page"] as const).map((x) => <button key={x} type="button" role="tab" aria-selected={tab === x} onClick={() => setTab(x)}>{t(`le.tab.${x}`)}</button>)}
          </div>
          {tab === "insert" && (
            <div role="tabpanel" aria-label={t("le.tab.insert")} className={s.panelBody}>
              <label className="search"><Icon name="search" size={16} /><input type="search" aria-label={t("le.findBlock")} placeholder={t("le.findBlock")} value={find} onChange={(e) => setFind(e.target.value)} /></label>
              {blockMenu().map((g) => {
                const list = g.items.filter((i) => (!i.feature || features.includes(i.feature as Feature)) && i.name.toLowerCase().includes(find.toLowerCase()));
                return list.length === 0 ? null : (
                  <div key={g.group}>
                    <p className={s.group}>{g.group}</p>
                    <div className={s.blocks}>
                      {list.map((i) => (
                        <button key={i.kind} type="button" className={s.blockBtn} title={i.hint} onClick={() => handle.current?.insert(i.kind)}>
                          <span aria-hidden="true" className={s.glyph}>{i.glyph}</span>{i.name}
                        </button>
                      ))}
                    </div>
                  </div>
                );
              })}
              <p className="muted" style={{ fontSize: 13 }}>{t("le.slashHint")}</p>
            </div>
          )}
          {tab === "style" && (
            <div role="tabpanel" aria-label={t("le.tab.style")} className={s.panelBody}>
              <p className={s.group}>{t("le.turnInto")}</p>
              <div role="radiogroup" aria-label={t("le.blockType")} className={s.blocks}>
                {TURN.map(([k, g, nameKey]) => (
                  <button key={k} type="button" role="radio" aria-checked={state.activeKind === k} className={`${s.blockBtn} ${s.radioBtn}`} disabled={!state.activeId} onClick={() => handle.current?.turnInto(k)}>
                    <span aria-hidden="true" className={s.glyph}>{g}</span>{t(nameKey)}
                  </button>
                ))}
              </div>
              <p className={s.group}>{t("le.selectedText")}</p>
              <div role="toolbar" aria-label={t("le.textStyle")} className={s.row2}>
                {([["bold", "B", "le.bold"], ["italic", "I", "le.italic"], ["strike", "S", "le.strike"], ["code", "</>", "le.inlineCode"]] as const).map(([f, g, n]) => (
                  <button key={f} type="button" className={s.blockBtn} style={{ minWidth: 44, justifyContent: "center" }} aria-label={t(n)} aria-pressed={state.formats[f]} onMouseDown={(e) => e.preventDefault()} onClick={() => handle.current?.format(f)}>{g}</button>
                ))}
              </div>
              <p className={s.group}>{t("le.block")}</p>
              <div className={s.row2}>
                <button type="button" className={s.blockBtn} onClick={() => handle.current?.duplicate()} disabled={!state.activeId}><Icon name="copy" size={15} strokeWidth={2} />{t("le.duplicate")}</button>
                <button type="button" className={s.blockBtn} onClick={() => handle.current?.move(-1)} disabled={!state.activeId}><Icon name="arrowUp" size={15} strokeWidth={2} />{t("cm.moveUp")}</button>
                <button type="button" className={s.blockBtn} onClick={() => handle.current?.move(1)} disabled={!state.activeId}><Icon name="arrowDown" size={15} strokeWidth={2} />{t("cm.moveDown")}</button>
                <button type="button" className={s.blockBtn} onClick={() => handle.current?.remove()} disabled={!state.activeId}><Icon name="trash" size={15} strokeWidth={2} />{t("le.deleteBlock")}</button>
              </div>
            </div>
          )}
          {tab === "page" && (
            <div role="tabpanel" aria-label={t("le.tab.page")} className={s.panelBody}>
              <div><p className={s.group}>{t("le.publishing")}</p>
                <p style={{ fontSize: 14 }}>{isDraft ? t("le.draftNote") : chip.tone === "accent" ? `${t("le.unpublishedChip")}${auto.savedAt ? `, ${t("sheet.saved", { time: timeOf(auto.savedAt) })}` : ""}` : t("home.allPublished")}</p>
                {lastPublished && <p className="muted" style={{ fontSize: 13 }}>{t("le.lastPublished", { day: dayLong(lastPublished), code: info.data?.code ?? "" })}</p>}
              </div>
              <div><p className={s.group}>{t("le.length")}</p><p style={{ fontSize: 14 }}>{t("le.lengthValue", { n: minutes, words: words.toLocaleString(locale()) })}</p></div>
              <div>
                <div className="row" style={{ justifyContent: "space-between", fontSize: 14 }}><span>{t("le.size")}</span><span className="muted">{t("le.sizeValue", { kb: Math.max(1, Math.round(bytes / 1024)), mb: Math.round(limit / 1_048_576) })}</span></div>
                <div aria-hidden="true" className={s.meter}><div style={{ width: `${Math.min(100, (bytes / limit) * 100)}%` }} /></div>
              </div>
              <div><p className={s.group}>{t("le.allowed")}</p><p className="muted" style={{ fontSize: 13 }}>{t("le.allowedLead")}</p></div>
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}
