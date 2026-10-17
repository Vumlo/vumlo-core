"use client";

import Link from "next/link";
import { useMemo, useRef, useState } from "react";
import { BlockEditor, type EditorHandle, type EditorState } from "@/components/doc/BlockEditor";
import { plainText } from "@/components/doc/BlockRenderer";
import { BLOCK_MENU, type InsertKind } from "@/components/doc/model";
import { Icon } from "@/components/Icon";
import { useEditorProfiles, usePublishItem, usePublishRevision, useItemRevision, useUpdateItem } from "@/lib/api/hooks/content";
import { useCourseInfo } from "@/lib/api/hooks/courses";
import { useAutosave } from "@/lib/autosave";
import { dayLong, timeOf } from "@/lib/format";
import { bodyOf, type Document, type Feature, type Item } from "@/lib/api/types";
import { AuthorBar } from "./AuthorBar";
import s from "./Author.module.css";

const FULL: Feature[] = ["headings", "text_styles", "links", "lists", "quote", "divider", "table", "code", "math", "mermaid", "video"];
const TURN: [InsertKind, string, string][] = [["paragraph", "T", "Text"], ["h1", "H1", "Heading 1"], ["h2", "H2", "Heading 2"], ["h3", "H3", "Heading 3"], ["quote", "“", "Quote"], ["bulleted", "•", "Bulleted"], ["numbered", "1.", "Numbered"], ["check", "✓", "Checklist"]];

export function LectureEditor({ item, courseId }: { item: Item; courseId: string }) {
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
  const chip = isDraft ? { text: "Draft, students can't see it", tone: "draft" as const } : changed ? { text: "Unpublished changes", tone: "accent" as const } : { text: "Published", tone: "ok" as const };

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
        <Link href={`/courses/${courseId}/items/${item.id}`} className="btn">Preview as student</Link>
        <button type="button" className="btn btn-primary" disabled={!canPublish || publishItem.pending || publishRevision.pending} onClick={publish}>{isDraft ? "Publish" : "Publish changes"}</button>
      </AuthorBar>
      {!isDraft && chip.tone === "accent" && lastPublished && <p className="muted" style={{ margin: "-4px 2px 16px", fontSize: 14 }}>Students see the version published on {dayLong(lastPublished)} until you publish these changes.</p>}
      <div className={s.shell}>
        <div className={s.canvas}>
          <div className={s.sheet}>
            <div className="row" style={{ gap: 8, marginBottom: 10 }}>
              <span aria-hidden="true" style={{ display: "flex", alignItems: "center", justifyContent: "center", width: 26, height: 26, borderRadius: 7, background: "var(--lec-tile)", color: "var(--lec-fg)" }}><Icon name="lecture" size={15} strokeWidth={2} /></span>
              <span style={{ fontSize: 14, fontWeight: 600, color: "var(--lec-fg)" }}>Lecture</span>
              <span className="muted" style={{ fontSize: 14 }}>About {minutes} min read</span>
            </div>
            <label htmlFor="title" className="sr-only">Title</label>
            <input id="title" className={s.titleInput} value={title} onChange={(e) => setTitle(e.target.value)} aria-invalid={!title.trim()} />
            {!title.trim() && <p className="hint" style={{ color: "var(--danger)", marginTop: 6 }}>A lecture needs a title.</p>}
            <div style={{ marginTop: 16, paddingLeft: 34 }}>
              <BlockEditor handle={handle} value={initial} onChange={setDoc} features={features} label="Lecture content" mode="float" minHeight={420} onState={setState} />
            </div>
          </div>
          <p className="muted" style={{ margin: "12px 2px 0", fontSize: 13 }}>Images and file attachments come later, with file uploads.</p>
        </div>

        <aside aria-label="Editor panel" className={`card ${s.panel}`}>
          <div role="tablist" aria-label="Panel sections" className={s.panelTabs}>
            {(["insert", "style", "page"] as const).map((t) => <button key={t} type="button" role="tab" aria-selected={tab === t} onClick={() => setTab(t)}>{t[0]!.toUpperCase() + t.slice(1)}</button>)}
          </div>
          {tab === "insert" && (
            <div role="tabpanel" aria-label="Insert" className={s.panelBody}>
              <label className="search"><Icon name="search" size={16} /><input type="search" aria-label="Find a block" placeholder="Find a block" value={find} onChange={(e) => setFind(e.target.value)} /></label>
              {BLOCK_MENU.map((g) => {
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
              <p className="muted" style={{ fontSize: 13 }}>Type / in the text to insert without leaving the keyboard. The list follows your institution&apos;s lecture profile.</p>
            </div>
          )}
          {tab === "style" && (
            <div role="tabpanel" aria-label="Style" className={s.panelBody}>
              <p className={s.group}>Turn this block into</p>
              <div role="radiogroup" aria-label="Block type" className={s.blocks}>
                {TURN.map(([k, g, n]) => (
                  <button key={k} type="button" role="radio" aria-checked={state.activeKind === k} className={`${s.blockBtn} ${s.radioBtn}`} disabled={!state.activeId} onClick={() => handle.current?.turnInto(k)}>
                    <span aria-hidden="true" className={s.glyph}>{g}</span>{n}
                  </button>
                ))}
              </div>
              <p className={s.group}>Selected text</p>
              <div role="toolbar" aria-label="Text style" className={s.row2}>
                {([["bold", "B", "Bold"], ["italic", "I", "Italic"], ["strike", "S", "Strikethrough"], ["code", "</>", "Inline code"]] as const).map(([f, g, n]) => (
                  <button key={f} type="button" className={s.blockBtn} style={{ minWidth: 44, justifyContent: "center" }} aria-label={n} aria-pressed={state.formats[f]} onMouseDown={(e) => e.preventDefault()} onClick={() => handle.current?.format(f)}>{g}</button>
                ))}
              </div>
              <p className={s.group}>Block</p>
              <div className={s.row2}>
                <button type="button" className={s.blockBtn} onClick={() => handle.current?.duplicate()} disabled={!state.activeId}><Icon name="copy" size={15} strokeWidth={2} />Duplicate</button>
                <button type="button" className={s.blockBtn} onClick={() => handle.current?.move(-1)} disabled={!state.activeId}><Icon name="arrowUp" size={15} strokeWidth={2} />Move up</button>
                <button type="button" className={s.blockBtn} onClick={() => handle.current?.move(1)} disabled={!state.activeId}><Icon name="arrowDown" size={15} strokeWidth={2} />Move down</button>
                <button type="button" className={s.blockBtn} onClick={() => handle.current?.remove()} disabled={!state.activeId}><Icon name="trash" size={15} strokeWidth={2} />Delete block</button>
              </div>
            </div>
          )}
          {tab === "page" && (
            <div role="tabpanel" aria-label="Page" className={s.panelBody}>
              <div><p className={s.group}>Publishing</p>
                <p style={{ fontSize: 14 }}>{isDraft ? "Draft, not visible to students." : chip.tone === "accent" ? `Unpublished changes${auto.savedAt ? `, saved ${timeOf(auto.savedAt)}` : ""}` : "Everything is published."}</p>
                {lastPublished && <p className="muted" style={{ fontSize: 13 }}>Last published {dayLong(lastPublished)}. Students of {info.data?.code} see that version.</p>}
              </div>
              <div><p className={s.group}>Length</p><p style={{ fontSize: 14 }}>About {minutes} min read, {words.toLocaleString("en-GB")} words</p></div>
              <div>
                <div className="row" style={{ justifyContent: "space-between", fontSize: 14 }}><span>Size</span><span className="muted">{Math.max(1, Math.round(bytes / 1024))} KB of {Math.round(limit / 1_048_576)} MB</span></div>
                <div aria-hidden="true" className={s.meter}><div style={{ width: `${Math.min(100, (bytes / limit) * 100)}%` }} /></div>
              </div>
              <div><p className={s.group}>Allowed content</p><p className="muted" style={{ fontSize: 13 }}>Lecture profile: text, headings, lists, quotes, tables, code, formulas, diagrams, videos and dividers. Your institution can narrow this list.</p></div>
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}
