# 0068. Formulas are drawn by KaTeX

Status: Accepted
Amends: ADR-0042 (the web frontend stack gains one dependency)

## Context
Formula blocks and inline formulas (ADR-0025, ADR-0027) are stored as LaTeX source. The web app drew
them with a small parser of its own that knew `\frac`, `\sqrt`, a few symbols and the common
operators. Anything else was shown as typed: `\int` was text, `\sum` had no limits, a matrix or a
`\begin{aligned}` block could not be written. Lessons and answers in maths and physics need the real
language, and a hand-written subset will always trail it. `web/AGENTS.md` forbids a new dependency
without an ADR, and the comment on the old renderer said a full renderer needed one.

## Options
1. **KaTeX.** Synchronous, draws HTML with its own CSS and fonts, covers nearly all of LaTeX maths,
   ships its types, and works offline, which the desktop app needs (ADR-0056, ADR-0057). Cost: about
   280 KB of script and the font files.
2. **Temml.** Draws MathML, so it is small and has no fonts. Cost: it relies on the browser's MathML
   support, looks plainer, and has fewer commands.
3. **MathJax.** The widest coverage. Cost: over 1 MB, asynchronous, and heavy to bundle for the
   static export.

## Decision
Option 1.
1. **Dependency.** `katex` 0.19.0, pinned exactly in `web/package.json`, with no other package: the
   release carries its own types. A version bump needs an ADR like any other (`web/AGENTS.md`).
2. **Where.** `components/doc/Math.tsx` is the one place that touches it. `Formula` loads the library
   with a dynamic import on the first formula, so a page without one does not carry it, and shows the
   source until it is there.
3. **Options.** `trust: false` (no `\href`, no raw HTML from a formula), `throwOnError: false` so a
   formula that cannot be read is shown in the error colour with its source instead of breaking the
   page, and the default output, which adds MathML for screen readers.
4. **Style.** The stylesheet and fonts of KaTeX are imported next to the component and bundled by
   Next, so nothing is fetched from a CDN. Colours come from `currentColor` and the `--danger`
   token, so both themes work.
5. **The old renderer is deleted**, with its styles. There is no fallback to it.

## Consequences
- Stored documents do not change: the source is the same LaTeX.
- The editor still shows the source of an inline formula while editing and the drawn formula in the
  preview of a formula block.
- `pnpm-lock.yaml` has to be refreshed with `pnpm install`; `package-lock.json` (used by `npm ci` in
  the Docker image) is updated in the same change.
