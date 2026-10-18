import { translate } from "@/lib/i18n/core";
import type { MessageKey } from "@/lib/i18n/core";
import type { Block, CodeLanguage, Document, Inline, Styles } from "@/lib/api/types";

export type TextBlockType = "paragraph" | "heading" | "quote" | "bulleted_list_item" | "numbered_list_item" | "check_list_item";

export const TEXT_TYPES: TextBlockType[] = ["paragraph", "heading", "quote", "bulleted_list_item", "numbered_list_item", "check_list_item"];

export const isTextBlock = (b: Block): b is Extract<Block, { type: TextBlockType }> => (TEXT_TYPES as string[]).includes(b.type);

let counter = 0;
export function newId(): string {
  counter += 1;
  return `b${Date.now().toString(36)}${counter.toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

export const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** Inline runs as the HTML the editable element shows. */
export function inlineToHtml(runs: Inline[] | undefined): string {
  return (runs ?? [])
    .map((r) => {
      if (r.type === "math") return `<span data-math="${esc(r.source)}" contenteditable="false" class="inline-math">${esc(r.source)}</span>`;
      if (r.type === "link") return `<a href="${esc(r.href)}" data-link="1">${esc(r.content.map((c) => c.text).join(""))}</a>`;
      let h = esc(r.text);
      const st = r.styles ?? {};
      if (st.code) h = `<code>${h}</code>`;
      if (st.strike) h = `<s>${h}</s>`;
      if (st.italic) h = `<em>${h}</em>`;
      if (st.bold) h = `<strong>${h}</strong>`;
      return h;
    })
    .join("");
}

function sameStyles(a: Styles, b: Styles) {
  return !!a.bold === !!b.bold && !!a.italic === !!b.italic && !!a.strike === !!b.strike && !!a.code === !!b.code;
}

/** Reads the editable element back into inline runs: adjacent text with the same styles is merged. */
export function htmlToInline(root: Node): Inline[] {
  const out: Inline[] = [];
  const push = (text: string, styles: Styles) => {
    if (!text) return;
    const last = out[out.length - 1];
    if (last && last.type === "text" && sameStyles(last.styles ?? {}, styles)) last.text += text;
    else out.push({ type: "text", text, ...(Object.keys(styles).length ? { styles: { ...styles } } : {}) });
  };
  const walk = (node: Node, styles: Styles) => {
    node.childNodes.forEach((n) => {
      if (n.nodeType === Node.TEXT_NODE) return push((n.textContent ?? "").replace(/​/g, ""), styles);
      if (!(n instanceof HTMLElement)) return;
      const tag = n.tagName.toLowerCase();
      if (n.dataset.math !== undefined) { out.push({ type: "math", source: n.dataset.math }); return; }
      if (tag === "br") return;
      if (tag === "a") {
        const href = n.getAttribute("href") ?? "";
        const text = n.textContent ?? "";
        if (/^https?:\/\//i.test(href) && text) out.push({ type: "link", href, content: [{ type: "text", text }] });
        else push(text, styles);
        return;
      }
      const next: Styles = { ...styles };
      if (tag === "strong" || tag === "b") next.bold = true;
      if (tag === "em" || tag === "i") next.italic = true;
      if (tag === "s" || tag === "strike" || tag === "del") next.strike = true;
      if (tag === "code") next.code = true;
      walk(n, next);
    });
  };
  walk(root, {});
  return out;
}

export const textOf = (runs: Inline[] | undefined) =>
  (runs ?? []).map((r) => (r.type === "text" ? r.text : r.type === "link" ? r.content.map((c) => c.text).join("") : r.source)).join("");

export function textBlock(type: TextBlockType, content: Inline[] = [], level: 1 | 2 | 3 = 2): Block {
  const id = newId();
  switch (type) {
    case "heading": return { id, type, props: { level }, content };
    case "check_list_item": return { id, type, props: { checked: false }, content };
    default: return { id, type, content };
  }
}

export type InsertKind = "paragraph" | "h1" | "h2" | "h3" | "quote" | "bulleted" | "numbered" | "check" | "table" | "code" | "math" | "mermaid" | "video" | "divider";

export function makeBlock(kind: InsertKind): Block {
  const id = newId();
  switch (kind) {
    case "paragraph": return textBlock("paragraph");
    case "h1": return textBlock("heading", [], 1);
    case "h2": return textBlock("heading", [], 2);
    case "h3": return textBlock("heading", [], 3);
    case "quote": return textBlock("quote");
    case "bulleted": return textBlock("bulleted_list_item");
    case "numbered": return textBlock("numbered_list_item");
    case "check": return textBlock("check_list_item");
    case "table": return { id, type: "table", props: { header_row: true, rows: [[[], []], [[], []]] } };
    case "code": return { id, type: "code", props: { language: "python" as CodeLanguage, source: "" } };
    case "math": return { id, type: "math", props: { source: "" } };
    case "mermaid": return { id, type: "mermaid", props: { source: "graph TD\n  A --> B" } };
    case "video": return { id, type: "video", props: { provider: "youtube", video_id: "" } };
    case "divider": return { id, type: "divider" };
  }
}

/** The kind of a block as the "turn into" control names it. */
export function kindOfBlock(b: Block): InsertKind {
  switch (b.type) {
    case "heading": return b.props.level === 1 ? "h1" : b.props.level === 3 ? "h3" : "h2";
    case "bulleted_list_item": return "bulleted";
    case "numbered_list_item": return "numbered";
    case "check_list_item": return "check";
    case "paragraph": return "paragraph";
    default: return b.type;
  }
}

export const emptyDoc = (): Document => ({ v: 1, blocks: [textBlock("paragraph")] });

/** Parses a YouTube or Vimeo address into the provider and id a video block stores (ADR-0027). */
export function parseVideo(input: string): { provider: "youtube" | "vimeo"; video_id: string } | null {
  const t = input.trim();
  const yt = /(?:youtube\.com\/(?:watch\?v=|embed\/|shorts\/)|youtu\.be\/)([A-Za-z0-9_-]{11})/.exec(t) ?? /^([A-Za-z0-9_-]{11})$/.exec(t);
  if (yt) return { provider: "youtube", video_id: yt[1]! };
  const vm = /vimeo\.com\/(?:video\/)?(\d{1,12})/.exec(t);
  return vm ? { provider: "vimeo", video_id: vm[1]! } : null;
}

type MenuItem = { kind: InsertKind; glyph: string; name: string; hint: string; feature?: string };

/** The insert menu in the current language. A function, so a language change shows at once. */
export function blockMenu(): { group: string; items: MenuItem[] }[] {
  const item = (kind: InsertKind, glyph: string, key: string, feature?: string): MenuItem => ({
    kind, glyph, name: translate(`blk.${key}` as MessageKey), hint: translate(`blk.${key}.hint` as MessageKey), ...(feature ? { feature } : {}),
  });
  return [
    { group: translate("blk.group.text"), items: [
      item("paragraph", "T", "text"),
      item("h1", "H1", "h1", "headings"),
      item("h2", "H2", "h2", "headings"),
      item("h3", "H3", "h3", "headings"),
      item("quote", "“", "quote", "quote"),
    ] },
    { group: translate("blk.group.lists"), items: [
      item("bulleted", "•", "bulleted", "lists"),
      item("numbered", "1.", "numbered", "lists"),
      item("check", "✓", "check", "lists"),
    ] },
    { group: translate("blk.group.insert"), items: [
      item("table", "≡", "table", "table"),
      item("code", "{ }", "code", "code"),
      item("math", "∑", "math", "math"),
      item("mermaid", "→", "mermaid", "mermaid"),
      item("video", "▶", "video", "video"),
      item("divider", "—", "divider", "divider"),
    ] },
  ];
}
