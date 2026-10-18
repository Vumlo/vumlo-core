"use client";

import { Fragment, useState, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import type { Block, CodeLanguage, Document, Inline } from "@/lib/api/types";
import { Formula } from "./Math";
import { highlight } from "./highlight";
import { translate } from "@/lib/i18n/core";
import s from "./Doc.module.css";

const LANGUAGE_NAMES: Partial<Record<CodeLanguage, string>> = {
  text: "Plain text", python: "Python", javascript: "JavaScript", typescript: "TypeScript", java: "Java", c: "C", cpp: "C++", csharp: "C#", rust: "Rust",
  go: "Go", sql: "SQL", bash: "Bash", html: "HTML", css: "CSS", json: "JSON", yaml: "YAML", markdown: "Markdown", latex: "LaTeX", kotlin: "Kotlin", swift: "Swift", php: "PHP",
};
export const languageName = (l: CodeLanguage) => (l === "text" ? translate("be.plainText") : LANGUAGE_NAMES[l] ?? l);

export function InlineRuns({ runs }: { runs: Inline[] | undefined }) {
  return (
    <>
      {(runs ?? []).map((r, i) => {
        if (r.type === "math") return <Formula key={i} source={r.source} />;
        if (r.type === "link") {
          const safe = /^https?:\/\//i.test(r.href);
          const label = r.content.map((c) => c.text).join("");
          return safe ? <a key={i} href={r.href} target="_blank" rel="noopener noreferrer">{label}</a> : <Fragment key={i}>{label}</Fragment>;
        }
        let node: ReactNode = r.text;
        const st = r.styles ?? {};
        if (st.code) node = <code>{node}</code>;
        if (st.strike) node = <s>{node}</s>;
        if (st.italic) node = <em>{node}</em>;
        if (st.bold) node = <strong>{node}</strong>;
        return <Fragment key={i}>{node}</Fragment>;
      })}
    </>
  );
}

export function CodeBlock({ source, language }: { source: string; language: CodeLanguage }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(source);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access can be denied; nothing else to do.
    }
  }
  return (
    <figure className={s.code}>
      <figcaption>
        <span>{languageName(language)}</span>
        <button type="button" className={s.copy} onClick={copy}>
          <Icon name={copied ? "check" : "copy"} size={14} strokeWidth={2} />
          {copied ? translate("be.copied") : translate("be.copy")}
        </button>
      </figcaption>
      <pre><code>{highlight(source, language).map((t, i) => <span key={i} className={t.kind === "plain" ? undefined : s[t.kind]}>{t.text}</span>)}</code></pre>
    </figure>
  );
}

function Video({ provider, id }: { provider: "youtube" | "vimeo"; id: string }) {
  const [playing, setPlaying] = useState(false);
  const name = provider === "youtube" ? "YouTube" : "Vimeo";
  const src = provider === "youtube" ? `https://www.youtube-nocookie.com/embed/${id}?autoplay=1` : `https://player.vimeo.com/video/${id}?autoplay=1`;
  return (
    <>
      <div className={s.video}>
        {playing ? (
          <iframe src={src} title={translate("be.videoFrom", { name })} allow="autoplay; fullscreen; picture-in-picture" allowFullScreen />
        ) : (
          <>
            <button type="button" className={s.play} aria-label={translate("be.play", { name })} onClick={() => setPlaying(true)}>
              <svg width="26" height="26" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5.5v13l11-6.5z" /></svg>
            </button>
            <span style={{ fontSize: 15, fontWeight: 500 }}>{translate("be.videoFrom", { name })}</span>
          </>
        )}
      </div>
      <p className={s.videoNote}>{translate("be.videoNote", { name })}</p>
    </>
  );
}

type ListKind = "bulleted_list_item" | "numbered_list_item" | "check_list_item";
const isList = (b: Block): b is Extract<Block, { type: ListKind }> => b.type === "bulleted_list_item" || b.type === "numbered_list_item" || b.type === "check_list_item";

function List({ items }: { items: Extract<Block, { type: ListKind }>[] }) {
  const kind = items[0]!.type;
  const body = items.map((b) => (
    <li key={b.id} className={b.type === "check_list_item" ? s.check : undefined}>
      {b.type === "check_list_item" && <input type="checkbox" checked={b.props.checked} readOnly aria-label={b.props.checked ? translate("be.done") : translate("be.notDone")} />}
      <span><InlineRuns runs={b.content} /></span>
      {b.children && b.children.length > 0 && <Blocks blocks={b.children} />}
    </li>
  ));
  return kind === "numbered_list_item" ? <ol>{body}</ol> : <ul>{body}</ul>;
}

export function Blocks({ blocks }: { blocks: Block[] }) {
  const out: ReactNode[] = [];
  for (let i = 0; i < blocks.length; i++) {
    const b = blocks[i]!;
    if (isList(b)) {
      const run = [b];
      while (i + 1 < blocks.length && blocks[i + 1]!.type === b.type) run.push(blocks[++i] as typeof b);
      out.push(<List key={b.id} items={run} />);
      continue;
    }
    switch (b.type) {
      case "paragraph":
        out.push(<p key={b.id}><InlineRuns runs={b.content} /></p>);
        break;
      case "heading":
        out.push(b.props.level === 3 ? <h3 key={b.id} id={b.id}><InlineRuns runs={b.content} /></h3> : <h2 key={b.id} id={b.id}><InlineRuns runs={b.content} /></h2>);
        break;
      case "quote":
        out.push(<blockquote key={b.id}><p><InlineRuns runs={b.content} /></p></blockquote>);
        break;
      case "divider":
        out.push(<hr key={b.id} />);
        break;
      case "code":
        out.push(<CodeBlock key={b.id} source={b.props.source} language={b.props.language} />);
        break;
      case "math":
        out.push(<Formula key={b.id} source={b.props.source} block />);
        break;
      case "mermaid":
        out.push(<div key={b.id} className={s.mermaid}><span className="muted">{translate("be.diagramSource")}</span><pre>{b.props.source}</pre></div>);
        break;
      case "video":
        out.push(<Video key={b.id} provider={b.props.provider} id={b.props.video_id} />);
        break;
      case "table": {
        const [head, ...rows] = b.props.rows;
        out.push(
          <div key={b.id} className={s.tableWrap}>
            <table className={s.table}>
              {b.props.header_row && head && <thead><tr>{head.map((c, k) => <th key={k} scope="col"><InlineRuns runs={c} /></th>)}</tr></thead>}
              <tbody>
                {(b.props.header_row ? rows : b.props.rows).map((r, ri) => <tr key={ri}>{r.map((c, k) => <td key={k}><InlineRuns runs={c} /></td>)}</tr>)}
              </tbody>
            </table>
          </div>,
        );
        break;
      }
    }
  }
  return <>{out}</>;
}

/** Renders a stored block document (ADR-0025). `size` is the body font size in px. */
export function BlockRenderer({ doc, size = 17 }: { doc: Document; size?: number }) {
  return (
    <div className={s.doc} style={{ ["--doc-size" as string]: `${size}px` }}>
      <Blocks blocks={doc.blocks} />
    </div>
  );
}

/** The plain text of a document, for word counts and previews. */
export function plainText(doc: Document): string {
  const text = (runs: Inline[] | undefined) => (runs ?? []).map((r) => (r.type === "text" ? r.text : r.type === "link" ? r.content.map((c) => c.text).join("") : "")).join("");
  const walk = (blocks: Block[]): string => blocks.map((b) => (b.type === "table" ? b.props.rows.flat().map(text).join(" ") : `${text("content" in b ? b.content : undefined)} ${"children" in b && b.children ? walk(b.children) : ""}`)).join(" ");
  return walk(doc.blocks);
}
