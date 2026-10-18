"use client";

import { useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode, type Ref } from "react";
import { Icon } from "@/components/Icon";
import { useT } from "@/lib/i18n";
import type { Block, CodeLanguage, Document, Feature } from "@/lib/api/types";
import { Formula } from "./Math";
import { languageName } from "./BlockRenderer";
import { blockMenu, esc, htmlToInline, inlineToHtml, isTextBlock, kindOfBlock, makeBlock, newId, parseVideo, textBlock, textOf, type InsertKind, type TextBlockType } from "./model";
import s from "./BlockEditor.module.css";
import { translate } from "@/lib/i18n/core";

export type Format = "bold" | "italic" | "strike" | "code";

export type EditorHandle = {
  insert: (kind: InsertKind) => void;
  turnInto: (kind: InsertKind) => void;
  move: (delta: -1 | 1) => void;
  duplicate: () => void;
  remove: () => void;
  format: (f: Format) => void;
};

export type EditorState = { activeId: string | null; activeKind: InsertKind | null; formats: Record<Format, boolean> };

type Props = {
  value: Document;
  onChange: (doc: Document) => void;
  features: Feature[];
  /** The accessible name of the editor area. */
  label: string;
  /** "bar" shows a fixed toolbar; "float" shows a bubble over the selection and a gutter per block. */
  mode: "bar" | "float";
  minHeight?: number;
  toolbarRight?: ReactNode;
  handle?: Ref<EditorHandle>;
  onState?: (s: EditorState) => void;
  placeholder?: string;
  /** Start with the slash menu open on the first empty block, for demos of the menu. */
  autoFocus?: boolean;
};

const LANGUAGES: CodeLanguage[] = ["python", "javascript", "typescript", "java", "c", "cpp", "csharp", "rust", "go", "sql", "bash", "html", "css", "json", "yaml", "markdown", "text"];

const SHORTCUTS: [RegExp, InsertKind, string][] = [
  [/^# $/, "h1", "headings"], [/^## $/, "h2", "headings"], [/^### $/, "h3", "headings"], [/^[-*] $/, "bulleted", "lists"], [/^1\. $/, "numbered", "lists"],
  [/^> $/, "quote", "quote"], [/^\[\] $/, "check", "lists"], [/^```$/, "code", "code"], [/^---$/, "divider", "divider"],
];

type Focus = { id: string; at: "start" | "end" | number };

function setCaret(el: HTMLElement, at: Focus["at"]) {
  el.focus();
  const sel = window.getSelection();
  if (!sel) return;
  const range = document.createRange();
  if (at === "start" || at === "end") {
    range.selectNodeContents(el);
    range.collapse(at === "start");
  } else {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    let left = at;
    let node = walker.nextNode();
    range.selectNodeContents(el);
    range.collapse(false);
    while (node) {
      const len = node.textContent?.length ?? 0;
      if (left <= len) { range.setStart(node, left); range.collapse(true); break; }
      left -= len;
      node = walker.nextNode();
    }
  }
  sel.removeAllRanges();
  sel.addRange(range);
}

const caretText = (el: HTMLElement, where: "before" | "after") => {
  const sel = window.getSelection();
  if (!sel?.rangeCount) return null;
  const r = sel.getRangeAt(0);
  if (!el.contains(r.startContainer)) return null;
  const probe = document.createRange();
  probe.selectNodeContents(el);
  if (where === "before") probe.setEnd(r.startContainer, r.startOffset);
  else probe.setStart(r.endContainer, r.endOffset);
  return probe.toString().replace(/​/g, "");
};

function Marker({ type, n, checked, onToggle }: { type: TextBlockType; n: number; checked: boolean; onToggle: () => void }) {
  const t = useT();
  if (type === "bulleted_list_item") return <span className={s.marker} aria-hidden="true">•</span>;
  if (type === "numbered_list_item") return <span className={s.marker} aria-hidden="true">{n}.</span>;
  return (
    <span className={s.marker}>
      <input type="checkbox" checked={checked} onChange={onToggle} aria-label={checked ? t("be.done") : t("be.notDone")} />
    </span>
  );
}

function TextEditable({ block, rev, placeholder, onInput, onKeyDown, onPaste, onFocus }: {
  block: Extract<Block, { type: TextBlockType }>;
  rev: number;
  placeholder: string;
  onInput: (el: HTMLElement) => void;
  onKeyDown: (e: ReactKeyboardEvent<HTMLElement>, el: HTMLElement) => void;
  onPaste: (e: React.ClipboardEvent<HTMLElement>) => void;
  onFocus: () => void;
}) {
  // The editable element keeps its own DOM while typing; React only sets it when the block is (re)mounted.
  const [initial] = useState(() => ({ __html: inlineToHtml(block.content) }));
  return (
    <div
      key={`${block.id}:${rev}`}
      className={s.text}
      contentEditable
      suppressContentEditableWarning
      role="textbox"
      aria-multiline="true"
      aria-label={block.type === "heading" ? translate("be.heading", { n: block.props.level }) : translate("blk.text")}
      data-kind={block.type === "heading" ? "heading" : block.type === "quote" ? "quote" : "text"}
      data-level={block.type === "heading" ? block.props.level : undefined}
      data-placeholder={placeholder}
      data-editable={block.id}
      spellCheck
      onInput={(e) => onInput(e.currentTarget)}
      onKeyDown={(e) => onKeyDown(e, e.currentTarget)}
      onPaste={onPaste}
      onFocus={onFocus}
      dangerouslySetInnerHTML={initial}
    />
  );
}

export function BlockEditor({ value, onChange, features, label, mode, minHeight = 240, toolbarRight, handle, onState, placeholder, autoFocus }: Props) {
  const t = useT();
  const hint = placeholder ?? t("be.placeholder");
  const [blocks, setBlocks] = useState<Block[]>(() => (value.blocks.length ? value.blocks : [textBlock("paragraph")]));
  const [revs, setRevs] = useState<Record<string, number>>({});
  const [activeId, setActiveId] = useState<string | null>(null);
  const [slash, setSlash] = useState<{ id: string; filter: string; index: number } | null>(null);
  const [float, setFloat] = useState<{ left: number; top: number } | null>(null);
  const [pop, setPop] = useState<null | "link" | "math">(null);
  const [popValue, setPopValue] = useState("");
  const [gutterMenu, setGutterMenu] = useState<string | null>(null);
  const [formats, setFormats] = useState<Record<Format, boolean>>({ bold: false, italic: false, strike: false, code: false });
  const root = useRef<HTMLDivElement>(null);
  const focus = useRef<Focus | null>(null);
  const saved = useRef<Range | null>(null);
  const has = useCallback((f?: string) => !f || features.includes(f as Feature), [features]);
  const blocksRef = useRef(blocks);
  useEffect(() => { blocksRef.current = blocks; });

  const commit = useCallback((next: Block[], to?: Focus, bump?: string[]) => {
    setBlocks(next);
    blocksRef.current = next;
    if (bump?.length) setRevs((r) => { const n = { ...r }; for (const id of bump) n[id] = (n[id] ?? 0) + 1; return n; });
    if (to) focus.current = to;
    onChange({ v: 1, blocks: next });
  }, [onChange]);

  useLayoutEffect(() => {
    const f = focus.current;
    if (!f) return;
    focus.current = null;
    const wrap = root.current?.querySelector<HTMLElement>(`[data-block-id="${f.id}"]`);
    const el = wrap?.querySelector<HTMLElement>("[data-editable], textarea, input:not([type=checkbox])");
    if (el) {
      if (el.hasAttribute("data-editable")) setCaret(el, f.at);
      else el.focus();
    }
  });

  useEffect(() => {
    if (autoFocus) focus.current = { id: blocks[0]!.id, at: "start" };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const indexOf = (id: string) => blocksRef.current.findIndex((b) => b.id === id);
  const patch = (id: string, fn: (b: Block) => Block, to?: Focus, bump?: string[]) => commit(blocksRef.current.map((b) => (b.id === id ? fn(b) : b)), to, bump);

  const state = useMemo<EditorState>(() => {
    const b = blocks.find((x) => x.id === activeId);
    return { activeId, activeKind: b ? kindOfBlock(b) : null, formats };
  }, [blocks, activeId, formats]);
  useEffect(() => { onState?.(state); }, [state, onState]);

  // ---- structure ----
  const turnInto = useCallback((kind: InsertKind, id = activeId ?? blocksRef.current[0]?.id) => {
    if (!id) return;
    const i = indexOf(id);
    const old = blocksRef.current[i];
    if (!old) return;
    const made = makeBlock(kind);
    const keep = isTextBlock(old) && isTextBlock(made) ? old.content ?? [] : [];
    const next: Block = isTextBlock(made) ? ({ ...made, content: keep } as Block) : made;
    const list = blocksRef.current.slice();
    list[i] = { ...next, id: old.id } as Block;
    if (!isTextBlock(next) && i === list.length - 1) list.push(textBlock("paragraph"));
    commit(list, { id: old.id, at: "end" }, [old.id]);
    setSlash(null);
  }, [activeId, commit]);

  const insert = useCallback((kind: InsertKind) => {
    const made = makeBlock(kind);
    const at = activeId ? indexOf(activeId) : blocksRef.current.length - 1;
    const cur = blocksRef.current[at];
    const list = blocksRef.current.slice();
    if (cur && isTextBlock(cur) && textOf(cur.content) === "" && (cur.type === "paragraph")) {
      list[at] = made;
      if (!isTextBlock(made) && at === list.length - 1) list.push(textBlock("paragraph"));
    } else {
      list.splice(at + 1, 0, made);
      if (!isTextBlock(made) && at + 1 === list.length - 1) list.push(textBlock("paragraph"));
    }
    commit(list, { id: made.id, at: "start" });
    setSlash(null);
  }, [activeId, commit]);

  const move = useCallback((delta: -1 | 1) => {
    if (!activeId) return;
    const i = indexOf(activeId);
    const j = i + delta;
    if (j < 0 || j >= blocksRef.current.length) return;
    const list = blocksRef.current.slice();
    const [b] = list.splice(i, 1);
    list.splice(j, 0, b!);
    commit(list, { id: b!.id, at: "end" }, [b!.id]);
  }, [activeId, commit]);

  const duplicate = useCallback(() => {
    if (!activeId) return;
    const i = indexOf(activeId);
    const copy = { ...JSON.parse(JSON.stringify(blocksRef.current[i])), id: newId() } as Block;
    const list = blocksRef.current.slice();
    list.splice(i + 1, 0, copy);
    commit(list, { id: copy.id, at: "end" });
  }, [activeId, commit]);

  const remove = useCallback(() => {
    if (!activeId) return;
    const i = indexOf(activeId);
    let list = blocksRef.current.filter((b) => b.id !== activeId);
    if (list.length === 0) list = [textBlock("paragraph")];
    const to = list[Math.min(i, list.length - 1)]!;
    commit(list, { id: to.id, at: "start" });
    setActiveId(to.id);
  }, [activeId, commit]);

  // ---- inline formatting ----
  const syncFromSelection = useCallback(() => {
    const sel = window.getSelection();
    const el = sel?.anchorNode instanceof Node ? (sel.anchorNode.nodeType === 1 ? (sel.anchorNode as HTMLElement) : sel.anchorNode.parentElement)?.closest<HTMLElement>("[data-editable]") : null;
    if (!el) return;
    const id = el.dataset.editable!;
    const runs = htmlToInline(el);
    commit(blocksRef.current.map((b) => (b.id === id && isTextBlock(b) ? ({ ...b, content: runs } as Block) : b)));
  }, [commit]);

  const format = useCallback((f: Format) => {
    const cmd = { bold: "bold", italic: "italic", strike: "strikeThrough" } as const;
    if (f === "code") {
      const sel = window.getSelection();
      if (!sel?.rangeCount || sel.isCollapsed) return;
      const inCode = (sel.anchorNode?.parentElement?.closest("code")) !== null && sel.anchorNode?.parentElement?.closest("code") !== undefined;
      const text = sel.toString();
      document.execCommand("insertHTML", false, inCode ? esc(text) : `<code>${esc(text)}</code>`);
    } else {
      document.execCommand(cmd[f]);
    }
    syncFromSelection();
  }, [syncFromSelection]);

  useImperativeHandle(handle, () => ({ insert, turnInto: (k) => turnInto(k), move, duplicate, remove, format }), [insert, turnInto, move, duplicate, remove, format]);

  const readFormats = useCallback(() => {
    const q = (c: string) => { try { return document.queryCommandState(c); } catch { return false; } };
    const sel = window.getSelection();
    const code = !!sel?.anchorNode && ((sel.anchorNode.nodeType === 1 ? (sel.anchorNode as HTMLElement) : sel.anchorNode.parentElement)?.closest("code") ?? null) !== null;
    setFormats({ bold: q("bold"), italic: q("italic"), strike: q("strikeThrough"), code });
  }, []);

  useEffect(() => {
    const onSel = () => {
      const sel = window.getSelection();
      const r = root.current;
      if (!sel?.rangeCount || !r) return;
      const range = sel.getRangeAt(0);
      if (!r.contains(range.commonAncestorContainer)) { if (!pop) setFloat(null); return; }
      readFormats();
      const el = (range.startContainer.nodeType === 1 ? (range.startContainer as HTMLElement) : range.startContainer.parentElement)?.closest<HTMLElement>("[data-editable]");
      if (el) setActiveId(el.dataset.editable!);
      if (mode !== "float" || sel.isCollapsed || !el) { if (!pop) setFloat(null); return; }
      const rect = range.getBoundingClientRect();
      const box = r.getBoundingClientRect();
      setFloat({ left: rect.left + rect.width / 2 - box.left, top: rect.top - box.top - 8 });
    };
    document.addEventListener("selectionchange", onSel);
    return () => document.removeEventListener("selectionchange", onSel);
  }, [mode, pop, readFormats]);

  function openPop(kind: "link" | "math") {
    const sel = window.getSelection();
    saved.current = sel?.rangeCount ? sel.getRangeAt(0).cloneRange() : null;
    setPopValue("");
    setPop(kind);
  }
  function applyPop() {
    const sel = window.getSelection();
    if (saved.current && sel) { sel.removeAllRanges(); sel.addRange(saved.current); }
    if (pop === "link") {
      const href = popValue.trim();
      if (/^https?:\/\//i.test(href)) document.execCommand("createLink", false, href);
    } else if (pop === "math" && popValue.trim()) {
      document.execCommand("insertHTML", false, `<span data-math="${esc(popValue.trim())}" contenteditable="false" class="inline-math">${esc(popValue.trim())}</span>&#8203;`);
    }
    setPop(null);
    syncFromSelection();
  }

  // ---- text block events ----
  function onInput(id: string, el: HTMLElement) {
    const b = blocksRef.current.find((x) => x.id === id);
    if (!b || !isTextBlock(b)) return;
    const text = (el.textContent ?? "").replace(/ /g, " ").replace(/​/g, "");
    if (b.type === "paragraph") {
      for (const [re, kind, feature] of SHORTCUTS) {
        if (re.test(text) && has(feature)) { el.textContent = ""; turnInto(kind, id); if (kind === "divider") { /* the divider replaced the block */ } return; }
      }
    }
    if (text.startsWith("/") && !/\s/.test(text) && textOf(b.content) !== text.slice(1)) {
      setSlash({ id, filter: text.slice(1).toLowerCase(), index: 0 });
    } else if (slash?.id === id && !text.startsWith("/")) {
      setSlash(null);
    }
    commit(blocksRef.current.map((x) => (x.id === id ? ({ ...x, content: htmlToInline(el) } as Block) : x)));
  }

  const slashItems = useMemo(() => {
    if (!slash) return [];
    return blockMenu().map((g) => ({ ...g, items: g.items.filter((it) => has(it.feature) && (it.name.toLowerCase().includes(slash.filter) || it.kind.includes(slash.filter))) })).filter((g) => g.items.length > 0);
  }, [slash, has]);
  const flatSlash = slashItems.flatMap((g) => g.items);

  function pickSlash(kind: InsertKind) {
    if (!slash) return;
    const id = slash.id;
    const i = indexOf(id);
    const old = blocksRef.current[i];
    if (old && isTextBlock(old)) patch(id, (b) => ({ ...b, content: [] }) as Block, undefined, [id]);
    turnInto(kind, id);
  }

  function onKeyDown(id: string, e: ReactKeyboardEvent<HTMLElement>, el: HTMLElement) {
    const b = blocksRef.current.find((x) => x.id === id);
    if (!b || !isTextBlock(b)) return;
    const mod = e.metaKey || e.ctrlKey;
    if (slash && slash.id === id) {
      if (e.key === "ArrowDown") { e.preventDefault(); setSlash({ ...slash, index: (slash.index + 1) % Math.max(1, flatSlash.length) }); return; }
      if (e.key === "ArrowUp") { e.preventDefault(); setSlash({ ...slash, index: (slash.index - 1 + flatSlash.length) % Math.max(1, flatSlash.length) }); return; }
      if (e.key === "Enter") { e.preventDefault(); const it = flatSlash[slash.index]; if (it) pickSlash(it.kind); return; }
      if (e.key === "Escape") { e.preventDefault(); setSlash(null); return; }
    }
    if (mod && e.key.toLowerCase() === "b" && has("text_styles")) { e.preventDefault(); format("bold"); return; }
    if (mod && e.key.toLowerCase() === "i" && has("text_styles")) { e.preventDefault(); format("italic"); return; }
    if (mod && e.key.toLowerCase() === "k" && has("links")) { e.preventDefault(); openPop("link"); return; }
    const list = blocksRef.current;
    const i = list.findIndex((x) => x.id === id);
    if (e.key === "Enter" && !e.shiftKey && !mod) {
      e.preventDefault();
      const sel = window.getSelection();
      if (!sel?.rangeCount) return;
      const range = sel.getRangeAt(0);
      const pre = document.createRange(); pre.selectNodeContents(el); pre.setEnd(range.startContainer, range.startOffset);
      const post = document.createRange(); post.selectNodeContents(el); post.setStart(range.endContainer, range.endOffset);
      const toRuns = (r: Range) => { const d = document.createElement("div"); d.appendChild(r.cloneContents()); return htmlToInline(d); };
      const before = toRuns(pre);
      const after = toRuns(post);
      const isList = b.type === "bulleted_list_item" || b.type === "numbered_list_item" || b.type === "check_list_item";
      if (isList && textOf(before) === "" && textOf(after) === "") { turnInto("paragraph", id); return; }
      const nextType: TextBlockType = b.type === "heading" ? "paragraph" : b.type;
      const fresh = textBlock(nextType, after);
      const copy = list.slice();
      copy[i] = { ...b, content: before } as Block;
      copy.splice(i + 1, 0, fresh);
      commit(copy, { id: fresh.id, at: "start" }, [id]);
      return;
    }
    if (e.key === "Backspace" && window.getSelection()?.isCollapsed && caretText(el, "before") === "") {
      if (b.type !== "paragraph") { e.preventDefault(); turnInto("paragraph", id); return; }
      const prev = list[i - 1];
      if (!prev) return;
      e.preventDefault();
      if (isTextBlock(prev)) {
        const len = textOf(prev.content).length;
        const merged = { ...prev, content: [...(prev.content ?? []), ...(b.content ?? [])] } as Block;
        const copy = list.slice();
        copy.splice(i - 1, 2, merged);
        commit(copy, { id: prev.id, at: len }, [prev.id]);
      } else if (prev.type === "divider" || textOf(b.content) === "") {
        const copy = list.filter((x) => x.id !== (prev.type === "divider" ? prev.id : id));
        commit(copy, { id: prev.type === "divider" ? id : prev.id, at: "end" });
      }
      return;
    }
    if (e.key === "ArrowUp" && !mod && window.getSelection()?.isCollapsed && caretText(el, "before") === "" && i > 0) {
      e.preventDefault();
      focus.current = { id: list[i - 1]!.id, at: "end" };
      setActiveId(list[i - 1]!.id);
      setBlocks((x) => x.slice());
      return;
    }
    if (e.key === "ArrowDown" && !mod && window.getSelection()?.isCollapsed && caretText(el, "after") === "" && i < list.length - 1) {
      e.preventDefault();
      focus.current = { id: list[i + 1]!.id, at: "start" };
      setActiveId(list[i + 1]!.id);
      setBlocks((x) => x.slice());
    }
  }

  function onPaste(e: React.ClipboardEvent<HTMLElement>) {
    e.preventDefault();
    const text = e.clipboardData.getData("text/plain").replace(/\r?\n+/g, " ");
    document.execCommand("insertText", false, text);
  }

  // ---- rendering ----
  const numbers = blocks.reduce<number[]>((acc, b, i) => { acc.push(b.type === "numbered_list_item" ? (blocks[i - 1]?.type === "numbered_list_item" ? (acc[i - 1] ?? 0) + 1 : 1) : 0); return acc; }, []);
  const bar = mode === "bar";
  const textStyles = has("text_styles");

  const tools = (
    <>
      {textStyles && (
        <>
          <button type="button" className={s.tool} aria-label={t("be.bold")} title={t("be.bold")} aria-pressed={formats.bold} onMouseDown={(e) => e.preventDefault()} onClick={() => format("bold")} style={{ fontWeight: 700 }}>B</button>
          <button type="button" className={s.tool} aria-label={t("be.italic")} title={t("be.italic")} aria-pressed={formats.italic} onMouseDown={(e) => e.preventDefault()} onClick={() => format("italic")} style={{ fontFamily: "Georgia, serif", fontStyle: "italic", fontWeight: 400 }}>I</button>
          <button type="button" className={s.tool} aria-label={t("be.strike")} title={t("be.strike")} aria-pressed={formats.strike} onMouseDown={(e) => e.preventDefault()} onClick={() => format("strike")} style={{ textDecoration: "line-through", fontWeight: 500 }}>S</button>
          <button type="button" className={s.tool} aria-label={t("be.inlineCode")} title={t("be.inlineCode")} aria-pressed={formats.code} onMouseDown={(e) => e.preventDefault()} onClick={() => format("code")}>
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M9 8l-4 4 4 4M15 8l4 4-4 4" /></svg>
          </button>
        </>
      )}
      {has("links") && <button type="button" className={s.tool} aria-label={t("be.link")} title={t("be.link")} onMouseDown={(e) => e.preventDefault()} onClick={() => openPop("link")}><Icon name="link" size={17} strokeWidth={1.9} /></button>}
      {has("math") && <button type="button" className={s.tool} aria-label={t("be.inlineFormula")} title={t("be.inlineFormula")} onMouseDown={(e) => e.preventDefault()} onClick={() => openPop("math")} style={{ fontWeight: 500 }}>∑</button>}
    </>
  );

  const popInput = pop && (
    <form className="row" style={{ gap: 6 }} onSubmit={(e) => { e.preventDefault(); applyPop(); }}>
      <input className={s.floatInput} autoFocus aria-label={pop === "link" ? t("be.linkAddress") : t("be.formulaLatex")} placeholder={pop === "link" ? "https://" : t("be.latexExample")} value={popValue} onChange={(e) => setPopValue(e.target.value)} onKeyDown={(e) => { if (e.key === "Escape") setPop(null); }} />
      <button type="submit" className="btn btn-primary" style={{ minHeight: 34 }}>{t("be.apply")}</button>
      <button type="button" className="btn" style={{ minHeight: 34 }} onClick={() => setPop(null)}>{t("common.cancel")}</button>
    </form>
  );

  return (
    <div ref={root} role="group" aria-label={label} className={s.wrap} onMouseDown={(e) => { if (!(e.target as HTMLElement).closest("[data-gutter-menu]")) setGutterMenu(null); }}>
      {bar && (
        <div role="toolbar" aria-label={t("be.formatting")} className={s.bar}>
          {pop ? popInput : (
            <>
              {tools}
              {has("lists") && (
                <>
                  <span aria-hidden="true" className={s.sep} />
                  <button type="button" className={s.tool} aria-label={t("be.bulleted")} title={t("be.bulleted")} onMouseDown={(e) => e.preventDefault()} onClick={() => turnInto("bulleted")}><svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" aria-hidden="true"><path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01" /></svg></button>
                  <button type="button" className={s.tool} aria-label={t("be.numbered")} title={t("be.numbered")} onMouseDown={(e) => e.preventDefault()} onClick={() => turnInto("numbered")}><svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" aria-hidden="true"><path d="M10 6h10M10 12h10M10 18h10M4 5l1.5-1v5M3.5 14.5c.3-.9 2.5-1 2.5.3 0 1-2.5 1.7-2.5 3.2H6" /></svg></button>
                </>
              )}
              {(has("code") || has("math")) && <span aria-hidden="true" className={s.sep} />}
              {has("code") && <button type="button" className={s.tool} aria-label={t("be.codeBlock")} title={t("be.codeBlock")} onMouseDown={(e) => e.preventDefault()} onClick={() => insert("code")}><svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M4 5h16v14H4zM9 10l-2 2 2 2M15 10l2 2-2 2" /></svg></button>}
              {has("math") && <button type="button" className={s.tool} aria-label={t("be.formulaBlock")} title={t("be.formulaBlock")} onMouseDown={(e) => e.preventDefault()} onClick={() => insert("math")}><svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M5 5h14M5 19h14M8 5l6 7-6 7" /></svg></button>}
              <span className={s.grow} />
              {toolbarRight}
            </>
          )}
        </div>
      )}

      {!bar && float && (
        <div role="toolbar" aria-label={t("be.formatSelection")} className={s.float} style={{ left: float.left, top: float.top }} onMouseDown={(e) => { if (!pop) e.preventDefault(); }}>
          {pop ? popInput : tools}
        </div>
      )}

      <div className={s.area} style={{ minHeight }}>
        {blocks.map((b, i) => {
          const rev = revs[b.id] ?? 0;
          const active = activeId === b.id;
          const gutter = !bar && (
            <div className={s.gutter} data-gutter-menu>
              <button type="button" className={s.gbtn} aria-label={t("be.addBlock")} onClick={() => { setActiveId(b.id); const made = makeBlock("paragraph"); const copy = blocksRef.current.slice(); copy.splice(i + 1, 0, made); commit(copy, { id: made.id, at: "start" }); setSlash({ id: made.id, filter: "", index: 0 }); }}>
                <Icon name="plus" size={16} strokeWidth={2} />
              </button>
              <button type="button" className={s.gbtn} aria-label={t("be.moveBlock")} aria-haspopup="menu" aria-expanded={gutterMenu === b.id} onClick={() => { setActiveId(b.id); setGutterMenu(gutterMenu === b.id ? null : b.id); }}>
                <Icon name="grip" size={16} strokeWidth={3} />
              </button>
              {gutterMenu === b.id && (
                <div role="menu" className={`popover ${s.menu}`} style={{ top: 30, left: 0 }}>
                  <button type="button" role="menuitem" className="menu-item" onClick={() => { duplicate(); setGutterMenu(null); }}><Icon name="copy" size={15} /> {t("le.duplicate")}</button>
                  <button type="button" role="menuitem" className="menu-item" onClick={() => { move(-1); setGutterMenu(null); }}><Icon name="arrowUp" size={15} /> {t("cm.moveUp")}</button>
                  <button type="button" role="menuitem" className="menu-item" onClick={() => { move(1); setGutterMenu(null); }}><Icon name="arrowDown" size={15} /> {t("cm.moveDown")}</button>
                  <button type="button" role="menuitem" className="menu-item" onClick={() => { remove(); setGutterMenu(null); }}><Icon name="trash" size={15} /> {t("le.deleteBlock")}</button>
                </div>
              )}
            </div>
          );
          if (isTextBlock(b)) {
            const isListItem = b.type === "bulleted_list_item" || b.type === "numbered_list_item" || b.type === "check_list_item";
            return (
              <div key={`${b.id}:${rev}`} className={s.block} data-block-id={b.id} data-active={active}>
                {gutter}
                <div className={isListItem ? s.list : s.content} style={{ width: "100%" }}>
                  {isListItem && <Marker type={b.type} n={numbers[i] ?? 0} checked={b.type === "check_list_item" && b.props.checked} onToggle={() => patch(b.id, (x) => (x.type === "check_list_item" ? { ...x, props: { checked: !x.props.checked } } : x))} />}
                  <TextEditable
                    block={b}
                    rev={rev}
                    placeholder={i === 0 && blocks.length === 1 && b.type === "paragraph" ? hint : b.type === "paragraph" ? "" : ""}
                    onInput={(el) => onInput(b.id, el)}
                    onKeyDown={(e, el) => onKeyDown(b.id, e, el)}
                    onPaste={onPaste}
                    onFocus={() => setActiveId(b.id)}
                  />
                </div>
                {slash?.id === b.id && flatSlash.length > 0 && (
                  <div role="listbox" aria-label={t("be.insertBlock")} className={s.slash} style={{ left: 34, top: "100%" }}>
                    {slashItems.map((g) => (
                      <div key={g.group} role="group" aria-label={g.group}>
                        <p className={s.slashGroup}>{g.group}</p>
                        {g.items.map((it) => (
                          <button key={it.kind} type="button" role="option" aria-selected={flatSlash[slash.index]?.kind === it.kind} className={s.slashItem} onMouseDown={(e) => e.preventDefault()} onClick={() => pickSlash(it.kind)}>
                            <span aria-hidden="true" className={s.glyph}>{it.glyph}</span>
                            <span><span className={s.itemName}>{it.name}</span><span className={s.itemHint}>{it.hint}</span></span>
                          </button>
                        ))}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          }
          return (
            <div key={b.id} className={s.block} data-block-id={b.id} data-active={active} onFocusCapture={() => setActiveId(b.id)} onClick={() => setActiveId(b.id)}>
              {gutter}
              <div className={s.content} style={{ width: "100%" }}>
                {b.type === "divider" && <hr className={s.hr} tabIndex={0} aria-label={t("blk.divider")} />}
                {b.type === "code" && (
                  <div className={s.card} data-active={active}>
                    <div className={s.cardHead}>
                      <label>{t("be.language")}{" "}
                        <select value={b.props.language} onChange={(e) => patch(b.id, (x) => (x.type === "code" ? { ...x, props: { ...x.props, language: e.target.value as CodeLanguage } } : x))}>
                          {LANGUAGES.map((l) => <option key={l} value={l}>{languageName(l)}</option>)}
                        </select>
                      </label>
                    </div>
                    <textarea className={s.code} aria-label={t("blk.code")} spellCheck={false} value={b.props.source} onChange={(e) => patch(b.id, (x) => (x.type === "code" ? { ...x, props: { ...x.props, source: e.target.value } } : x))} />
                  </div>
                )}
                {b.type === "math" && (
                  <div className={s.card} data-active={active}>
                    <div className={s.cardHead}><span>{t("be.formulaTitle")}</span><span>{t("be.pressEnter")}</span></div>
                    <div className={s.mathBody}>
                      <label className="sr-only" htmlFor={`m-${b.id}`}>{t("be.latexSource")}</label>
                      <input id={`m-${b.id}`} className={s.mathInput} value={b.props.source} placeholder="T(n) = 2T\left(\frac{n}{2}\right) + n" onChange={(e) => patch(b.id, (x) => (x.type === "math" ? { ...x, props: { source: e.target.value } } : x))} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); const n = blocksRef.current[blocksRef.current.findIndex((x) => x.id === b.id) + 1]; if (n) { focus.current = { id: n.id, at: "start" }; setBlocks((x) => x.slice()); } } }} />
                      <div className={s.preview}>{b.props.source ? <Formula source={b.props.source} /> : <span className="muted" style={{ fontSize: 14 }}>{t("be.previewHere")}</span>}</div>
                    </div>
                  </div>
                )}
                {b.type === "mermaid" && (
                  <div className={s.card} data-active={active}>
                    <div className={s.cardHead}><span>{t("be.diagramTitle")}</span></div>
                    <textarea className={s.code} aria-label={t("be.diagramSource")} spellCheck={false} value={b.props.source} onChange={(e) => patch(b.id, (x) => (x.type === "mermaid" ? { ...x, props: { source: e.target.value } } : x))} />
                  </div>
                )}
                {b.type === "video" && (
                  <div className={s.card} data-active={active}>
                    <div className={s.cardHead}><span>{t("be.videoTitle")}</span><span>{b.props.video_id ? t("be.videoId", { provider: b.props.provider === "youtube" ? "YouTube" : "Vimeo", id: b.props.video_id }) : ""}</span></div>
                    <div className={s.mathBody}>
                      <label className="sr-only" htmlFor={`v-${b.id}`}>{t("be.videoAddress")}</label>
                      <input id={`v-${b.id}`} className={s.mathInput} style={{ fontFamily: "inherit" }} placeholder={t("be.videoPaste")} defaultValue={b.props.video_id ? (b.props.provider === "youtube" ? `https://youtu.be/${b.props.video_id}` : `https://vimeo.com/${b.props.video_id}`) : ""} onBlur={(e) => { const p = parseVideo(e.target.value); if (p) patch(b.id, (x) => (x.type === "video" ? { ...x, props: p } : x)); }} />
                    </div>
                  </div>
                )}
                {b.type === "table" && (
                  <div className={s.card} data-active={active}>
                    <table className={s.cells}>
                      <tbody>
                        {b.props.rows.map((row, ri) => (
                          <tr key={ri}>
                            {row.map((cell, ci) => {
                              const Cell = b.props.header_row && ri === 0 ? "th" : "td";
                              return (
                                <Cell key={ci}>
                                  <input aria-label={t("be.cell", { row: ri + 1, col: ci + 1 })} value={textOf(cell)} onChange={(e) => patch(b.id, (x) => (x.type === "table" ? { ...x, props: { ...x.props, rows: x.props.rows.map((r, a) => r.map((c, z) => (a === ri && z === ci ? (e.target.value ? [{ type: "text" as const, text: e.target.value }] : []) : c))) } } : x))} />
                                </Cell>
                              );
                            })}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                    <div className={s.tableTools}>
                      <button type="button" className={s.mini} onClick={() => patch(b.id, (x) => (x.type === "table" ? { ...x, props: { ...x.props, rows: [...x.props.rows, x.props.rows[0]!.map(() => [])] } } : x))}>{t("be.addRow")}</button>
                      <button type="button" className={s.mini} onClick={() => patch(b.id, (x) => (x.type === "table" ? { ...x, props: { ...x.props, rows: x.props.rows.map((r) => [...r, []]) } } : x))}>{t("be.addColumn")}</button>
                      <button type="button" className={s.mini} disabled={b.props.rows.length <= 1} onClick={() => patch(b.id, (x) => (x.type === "table" ? { ...x, props: { ...x.props, rows: x.props.rows.slice(0, -1) } } : x))}>{t("be.removeRow")}</button>
                    </div>
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
