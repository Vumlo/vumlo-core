"use client";

import { initials } from "@/lib/format";
import { useT } from "@/lib/i18n";
import type { GradebookRow } from "@/lib/api/types";
import type { CellState } from "@/lib/grading";
import s from "./GradeWorkspace.module.css";

/** The students that pass the filter, each with a text mark of where their work stands. */
export function StudentRail({ rows, states, late, selected, onPick }: { rows: GradebookRow[]; states: Map<string, CellState>; late: ReadonlySet<string>; selected: string; onPick: (studentId: string) => void }) {
  const t = useT();
  const mark = { grade: t("gb.toGrade"), draft: t("gw.mDraft"), pub: t("gw.mPub"), none: t("gw.mNone") } satisfies Record<CellState, string>;
  return (
    <nav aria-label={t("gw.rail")} className={s.rail}>
      {rows.map((r) => {
        const st = states.get(r.user_id) ?? "none";
        return (
          <button key={r.user_id} type="button" className={s.railItem} aria-current={r.user_id === selected ? "true" : undefined} onClick={() => onPick(r.user_id)}>
            <span aria-hidden="true" className={s.avatar}>{initials(r.name, r.surname)}</span>
            <span className={s.railText}>
              <span className={s.railName}>{r.name} {r.surname}</span>
              <span className={s.marks}>
                <span className={s.mark} data-state={st}>{mark[st]}</span>
                {late.has(r.user_id) && <span className={s.lateMark}>{t("gb.late")}</span>}
              </span>
            </span>
          </button>
        );
      })}
    </nav>
  );
}
