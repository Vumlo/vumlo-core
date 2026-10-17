import type { Block, Document, Inline } from "@/lib/api/types";

let n = 0;
const bid = () => `b${++n}`;

type Run = string | { b?: string; i?: string; s?: string; code?: string; math?: string; link?: [string, string] };

/** Inline runs from a compact notation: plain strings, or one styled run. */
export function runs(...parts: Run[]): Inline[] {
  return parts.map((p): Inline => {
    if (typeof p === "string") return { type: "text", text: p };
    if (p.math) return { type: "math", source: p.math };
    if (p.link) return { type: "link", href: p.link[1], content: [{ type: "text", text: p.link[0] }] };
    if (p.b) return { type: "text", text: p.b, styles: { bold: true } };
    if (p.i) return { type: "text", text: p.i, styles: { italic: true } };
    if (p.s) return { type: "text", text: p.s, styles: { strike: true } };
    return { type: "text", text: p.code ?? "", styles: { code: true } };
  });
}

export const para = (...r: Run[]): Block => ({ id: bid(), type: "paragraph", content: runs(...r) });
export const quote = (...r: Run[]): Block => ({ id: bid(), type: "quote", content: runs(...r) });
export const heading = (level: 1 | 2 | 3, text: string): Block => ({ id: bid(), type: "heading", props: { level }, content: runs(text) });
export const bullet = (...r: Run[]): Block => ({ id: bid(), type: "bulleted_list_item", content: runs(...r) });
export const numbered = (...r: Run[]): Block => ({ id: bid(), type: "numbered_list_item", content: runs(...r) });
export const divider = (): Block => ({ id: bid(), type: "divider" });
export const code = (language: "python" | "text" | "sql", source: string): Block => ({ id: bid(), type: "code", props: { language, source } });
export const math = (source: string): Block => ({ id: bid(), type: "math", props: { source } });
export const video = (video_id: string): Block => ({ id: bid(), type: "video", props: { provider: "youtube", video_id } });
export const table = (rows: string[][]): Block => ({
  id: bid(),
  type: "table",
  props: { header_row: true, rows: rows.map((r) => r.map((c) => runs(c))) },
});

export const doc = (...blocks: Block[]): Document => ({ v: 1, blocks });
