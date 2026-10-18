"use client";

import { useState } from "react";
import { useT } from "@/lib/i18n";
import { useUpdateItem } from "@/lib/api/hooks/content";
import { useItemMeta } from "@/lib/api/hooks/courses";
import { isoToLocal, localToIso } from "@/lib/format";
import type { Item } from "@/lib/api/types";
import { Switch } from "./AuthorBar";

/** Points, deadline, attempts and late rules of an assignment (ADR-0021, ADR-0035). Saved with its own button. */
export function AssignmentSettings({ item, courseId, pointsLocked }: { item: Item; courseId: string; pointsLocked?: boolean }) {
  const t = useT();
  const a = item.assignment!;
  const meta = useItemMeta(courseId);
  const update = useUpdateItem(courseId);
  const noSubmission = a.type === "no_submission";
  const [points, setPoints] = useState(a.max_points);
  const [hasDue, setHasDue] = useState(!!a.due_at);
  const [due, setDue] = useState(a.due_at ? isoToLocal(a.due_at) : "");
  const [attempts, setAttempts] = useState(a.max_attempts == null ? "" : String(a.max_attempts));
  const [late, setLate] = useState(!!a.accept_late);
  const [lateUntil, setLateUntil] = useState(a.late_until ? isoToLocal(a.late_until) : "");
  const [onDeadline, setOnDeadline] = useState<string>(a.on_deadline ?? "keep_draft");
  const classOn = meta.data?.items[item.id]?.class_on;
  const [classDay, setClassDay] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const err = (f: string) => update.error?.errors.find((x) => x.field === f)?.message;

  async function save() {
    setSaved(false);
    const patch: Record<string, unknown> = { ...(pointsLocked ? {} : { max_points: points }), };
    if (!noSubmission) {
      if (hasDue && due) Object.assign(patch, { due_at: localToIso(due), on_deadline: onDeadline, accept_late: late, late_until: late && lateUntil ? localToIso(lateUntil) : null });
      else Object.assign(patch, { due_at: null, on_deadline: null, accept_late: null, late_until: null });
      Object.assign(patch, { max_attempts: attempts.trim() === "" ? null : Number(attempts) });
    }
    if (noSubmission && classDay !== null) patch.class_on = classDay || null;
    if (await update.run(item.id, patch)) setSaved(true);
  }

  return (
    <div className="stack" style={{ gap: 14 }}>
      {!pointsLocked && (
        <div className="field">
          <label htmlFor="as-points">{t("cm.points")}</label>
          <input id="as-points" className="input" style={{ maxWidth: 160 }} inputMode="decimal" value={points} onChange={(e) => { setPoints(e.target.value); setSaved(false); }} aria-invalid={!!err("max_points")} />
          {err("max_points") && <p className="hint" style={{ color: "var(--danger)" }}>{err("max_points")}</p>}
        </div>
      )}
      {noSubmission ? (
        <div className="field">
          <label htmlFor="as-class">{t("as.classDate")}</label>
          <input id="as-class" className="input" type="date" style={{ maxWidth: 200 }} value={classDay ?? (classOn ? isoToLocal(classOn).slice(0, 10) : "")} onChange={(e) => { setClassDay(e.target.value); setSaved(false); }} />
          <p className="hint">{t("as.classHint")}</p>
        </div>
      ) : (
        <>
          <div className="switchRow" style={{ display: "flex", justifyContent: "space-between", gap: 12, alignItems: "center" }}>
            <span style={{ fontSize: 14, fontWeight: 500 }}>{t("as.hasDeadline")}</span>
            <Switch checked={hasDue} onChange={(v) => { setHasDue(v); setSaved(false); }} label={t("as.hasDeadline")} />
          </div>
          {hasDue && (
            <>
              <div className="field">
                <label htmlFor="as-due">{t("as.deadline")}</label>
                <input id="as-due" className="input" type="datetime-local" value={due} onChange={(e) => { setDue(e.target.value); setSaved(false); }} aria-invalid={!!err("due_at")} />
                {err("due_at") && <p className="hint" style={{ color: "var(--danger)" }}>{err("due_at")}</p>}
              </div>
              <div className="field">
                <label htmlFor="as-dl">{t("as.atDeadline")}</label>
                <select id="as-dl" className="select" value={onDeadline} onChange={(e) => { setOnDeadline(e.target.value); setSaved(false); }}>
                  <option value="keep_draft">{t("as.keepDraft")}</option>
                  <option value="auto_submit">{t("as.autoSubmit")}</option>
                </select>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", gap: 12, alignItems: "center" }}>
                <span style={{ fontSize: 14, fontWeight: 500 }}>{t("as.acceptLate")}</span>
                <Switch checked={late} onChange={(v) => { setLate(v); setSaved(false); }} label={t("as.acceptLate")} />
              </div>
              {late && (
                <div className="field">
                  <label htmlFor="as-late">{t("as.lateUntil")}</label>
                  <input id="as-late" className="input" type="datetime-local" value={lateUntil} onChange={(e) => { setLateUntil(e.target.value); setSaved(false); }} />
                </div>
              )}
            </>
          )}
          <div className="field">
            <label htmlFor="as-att">{t("as.attempts")}</label>
            <input id="as-att" className="input" style={{ maxWidth: 160 }} inputMode="numeric" placeholder={t("as.unlimited")} value={attempts} onChange={(e) => { setAttempts(e.target.value); setSaved(false); }} aria-invalid={!!err("max_attempts")} />
            <p className="hint">{t("as.attemptsHint")}</p>
            {err("max_attempts") && <p className="hint" style={{ color: "var(--danger)" }}>{err("max_attempts")}</p>}
          </div>
        </>
      )}
      {update.error && !update.error.errors.length && <p role="alert" className="alert alert-danger">{update.error.detail ?? update.error.title}</p>}
      <div className="row" style={{ gap: 12 }}>
        <button type="button" className="btn btn-primary" onClick={save} disabled={update.pending}>{t("as.save")}</button>
        <span role="status" className="muted" style={{ fontSize: 14 }}>{saved ? t("cs.saved") : ""}</span>
      </div>
    </div>
  );
}
