"use client";

import { useEffect, useMemo, useState } from "react";
import "katex/dist/katex.min.css";

type Katex = typeof import("katex").default;

let loading: Promise<Katex> | null = null;
/** The library is fetched with the first formula, so a page without one does not carry it (ADR-0068). */
const load = () => (loading ??= import("katex").then((m) => m.default));

/** A formula in LaTeX, drawn by KaTeX. Until the library is there, and when the source cannot be read, the source is shown. */
export function Formula({ source, block = false }: { source: string; block?: boolean }) {
  const [katex, setKatex] = useState<Katex | null>(null);
  useEffect(() => {
    let live = true;
    void load().then((k) => live && setKatex(() => k));
    return () => { live = false; };
  }, []);
  const html = useMemo(
    () => katex?.renderToString(source, { displayMode: block, throwOnError: false, errorColor: "var(--danger)", trust: false }) ?? null,
    [katex, source, block],
  );
  const className = block ? "math math-block" : "math";
  return html === null
    ? <span className={className}>{source}</span>
    : <span className={className} dangerouslySetInnerHTML={{ __html: html }} />;
}
