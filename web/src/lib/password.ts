import { translate } from "./i18n/core";

export type Strength = { level: 0 | 1 | 2 | 3 | 4; label: string };

/** A rough hint only: the server applies the real password policy (ADR-0017). */
export function passwordStrength(p: string): Strength {
  if (p.length === 0) return { level: 0, label: "" };
  const words = p.trim().split(/\s+/).filter(Boolean).length;
  if (p.length < 8) return { level: 1, label: translate("password.short") };
  if (p.length < 12 && words < 3) return { level: 2, label: translate("password.longer") };
  if (p.length >= 16 || words >= 3) return { level: 4, label: translate("password.strong") };
  return { level: 3, label: translate("password.good") };
}
