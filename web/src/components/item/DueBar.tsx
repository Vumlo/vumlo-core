import type { ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { StatusPill } from "@/components/StatusPill";
import type { PillTone } from "@/lib/course";
import { dueSentence, relative } from "@/lib/format";
import { wholePoints } from "@/lib/points";
import { useT } from "@/lib/i18n";
import s from "./Item.module.css";

type Props = {
  due: string | null;
  open: boolean;
  points: string;
  pill: { label: string; tone: PillTone };
  danger?: boolean;
  /** Replaces the due sentence, for in-class work. */
  sentence?: ReactNode;
  /** True for work that carries no points ("not graded"). */
  ungraded?: boolean;
  children?: ReactNode;
};

/** The sticky strip under the title: deadline, points, state and the main action. */
export function DueBar({ due, open, points, pill, danger, sentence, ungraded, children }: Props) {
  const t = useT();
  return (
    <div className={s.bar}>
      <Icon name="clock" size={20} strokeWidth={1.9} style={{ flex: "none", color: danger ? "var(--danger)" : "var(--text)" }} />
      <p className={s.barText}>
        {sentence ?? (
          <>
            <span className={s.barStrong} style={{ color: danger ? "var(--danger)" : undefined }}>
              {due ? t(open ? "bar.due" : "bar.closed", { when: dueSentence(due) }) : t("due.none")}
            </span>
            {due && open && <span className="muted">, {relative(due)}</span>}
          </>
        )}
        <span className="muted">{ungraded ? t("bar.notGraded") : t("bar.worth", { points: wholePoints(points) })}</span>
      </p>
      <StatusPill tone={pill.tone}>{pill.label}</StatusPill>
      {children}
    </div>
  );
}
