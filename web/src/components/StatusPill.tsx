import type { PillTone } from "@/lib/course";

const CLASS: Record<PillTone, string> = {
  plain: "pill",
  strong: "pill pill-strong",
  accent: "pill pill-accent",
  ok: "pill pill-ok",
  danger: "pill pill-danger",
};

export function StatusPill({ tone, children, minWidth }: { tone: PillTone; children: React.ReactNode; minWidth?: number }) {
  return (
    <span className={CLASS[tone]} style={minWidth ? { minWidth } : undefined}>
      {children}
    </span>
  );
}
