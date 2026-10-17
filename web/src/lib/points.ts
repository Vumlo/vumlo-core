/**
 * Points are decimal strings with two decimals (ADR-0021). Arithmetic runs on integer
 * hundredths so a float never touches a score.
 */
export type Points = string;

export function toCents(p: Points): number {
  const m = /^(-?)(\d+)(?:\.(\d{1,2}))?$/.exec(p.trim());
  if (!m) return Number.NaN;
  const cents = Number(m[2]) * 100 + Number((m[3] ?? "").padEnd(2, "0"));
  return m[1] ? -cents : cents;
}

export function fromCents(c: number): Points {
  const sign = c < 0 ? "-" : "";
  const abs = Math.abs(Math.round(c));
  return `${sign}${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")}`;
}

/** "7.5" becomes "7.50". Anything that is not a valid score stays as typed. */
export function normalizePoints(p: Points): Points {
  const c = toCents(p);
  return Number.isNaN(c) ? p : fromCents(c);
}

export function sumPoints(list: Points[]): Points {
  return fromCents(list.reduce((a, p) => a + (toCents(p) || 0), 0));
}

/** Whole percent of `score` in `max`, or null when max is zero. */
export function percent(score: Points, max: Points): number | null {
  const m = toCents(max);
  return m > 0 ? Math.round((toCents(score) / m) * 100) : null;
}

/** A score field accepts "0" up to max with at most two decimals (ADR-0021). */
export function validScore(input: string, max: Points): boolean {
  if (!/^\d+(\.\d{1,2})?$/.test(input.trim())) return false;
  return toCents(input) <= toCents(max);
}

/** "30.00" reads "30" in a sentence such as "worth 30 points"; "7.50" stays. */
export function wholePoints(p: Points): string {
  return p.endsWith(".00") ? p.slice(0, -3) : p;
}
