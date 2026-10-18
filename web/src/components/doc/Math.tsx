import type { ReactNode } from "react";

/**
 * A small LaTeX subset for formulas in lessons: letters, digits, \frac, ^ and _, \left/\right,
 * \log and friends, a few symbols. Anything else is shown as typed. A full renderer needs its own ADR (ADR-0042).
 */
const SYMBOLS: Record<string, string> = {
  cdot: "·", times: "×", le: "≤", leq: "≤", ge: "≥", geq: "≥", neq: "≠", approx: "≈", infty: "∞", sum: "∑", prod: "∏",
  pi: "π", alpha: "α", beta: "β", theta: "θ", lambda: "λ", mu: "μ", sigma: "σ", Theta: "Θ", Omega: "Ω", to: "→", pm: "±", in: "∈",
};
const OPERATORS = new Set(["log", "ln", "lg", "sin", "cos", "tan", "max", "min", "lim", "exp"]);

type Cursor = { s: string; i: number };

function group(c: Cursor): string {
  if (c.s[c.i] !== "{") return c.s[c.i++] ?? "";
  let depth = 0;
  const start = c.i + 1;
  while (c.i < c.s.length) {
    if (c.s[c.i] === "{") depth++;
    if (c.s[c.i] === "}") depth--;
    c.i++;
    if (depth === 0) break;
  }
  return c.s.slice(start, c.i - 1);
}

function parse(src: string, key = "m"): ReactNode[] {
  const c: Cursor = { s: src, i: 0 };
  const out: ReactNode[] = [];
  let n = 0;
  const k = () => `${key}${n++}`;
  const attach = (kind: "sup" | "sub", arg: string) => {
    const el = kind === "sup" ? <sup key={k()}>{parse(arg, `${key}${n}`)}</sup> : <sub key={k()}>{parse(arg, `${key}${n}`)}</sub>;
    out.push(el);
  };
  while (c.i < c.s.length) {
    const ch = c.s[c.i]!;
    if (ch === "\\") {
      c.i++;
      let name = "";
      while (/[a-zA-Z]/.test(c.s[c.i] ?? "")) name += c.s[c.i++];
      if (!name) { out.push(c.s[c.i++] ?? ""); continue; }
      if (name === "frac") {
        const a = group(c);
        const b = group(c);
        out.push(
          <span key={k()} className="frac">
            <span className="frac-n">{parse(a, `${key}${n}a`)}</span>
            <span>{parse(b, `${key}${n}b`)}</span>
          </span>,
        );
      } else if (name === "left" || name === "right") {
        out.push(c.s[c.i++] ?? "");
      } else if (name === "sqrt") {
        out.push(<span key={k()}>√<span style={{ borderTop: "1px solid currentColor" }}>{parse(group(c), `${key}${n}r`)}</span></span>);
      } else if (OPERATORS.has(name)) {
        out.push(<span key={k()} style={{ fontStyle: "normal" }}>{" "}{name}{" "}</span>);
      } else if (SYMBOLS[name]) {
        out.push(<span key={k()}>{SYMBOLS[name]}</span>);
      } else {
        out.push(`\\${name}`);
      }
    } else if (ch === "^") {
      c.i++;
      attach("sup", group(c));
    } else if (ch === "_") {
      c.i++;
      attach("sub", group(c));
    } else if (ch === "{" || ch === "}") {
      c.i++;
    } else if (/[a-zA-Z]/.test(ch)) {
      out.push(<i key={k()}>{ch}</i>);
      c.i++;
    } else {
      out.push(ch);
      c.i++;
    }
  }
  return out;
}

export function Formula({ source, block = false }: { source: string; block?: boolean }) {
  return (
    <span role="math" aria-label={source} className={block ? "math math-block" : "math"}>
      {parse(source)}
    </span>
  );
}
