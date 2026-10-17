"use client";

import { useState } from "react";
import { useUpdateItem } from "@/lib/api/hooks/content";
import { useItemMeta } from "@/lib/api/hooks/courses";
import { isoToLocal, localToIso } from "@/lib/format";
import type { Item } from "@/lib/api/types";
import { Switch } from "./AuthorBar";

/** Points, deadline, attempts and late rules of an assignment (ADR-0021, ADR-0035). Saved with its own button. */
export function AssignmentSettings({ item, courseId, pointsLocked }: { item: Item; courseId: string; pointsLocked?: boolean }) {
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
    if (noSubmission && classDay !== null) patch.class_on = classDay ? localToIso(`${classDay}T12:00`) : null;
    if (await update.run(item.id, patch)) setSaved(true);
  }

  return (
    <div className="stack" style={{ gap: 14 }}>
      {!pointsLocked && (
        <div className="field">
          <label htmlFor="as-points">Points</label>
          <input id="as-points" className="input" style={{ maxWidth: 160 }} inputMode="decimal" value={points} onChange={(e) => { setPoints(e.target.value); setSaved(false); }} aria-invalid={!!err("max_points")} />
          {err("max_points") && <p className="hint" style={{ color: "var(--danger)" }}>{err("max_points")}</p>}
        </div>
      )}
      {noSubmission ? (
        <div className="field">
          <label htmlFor="as-class">Date in class</label>
          <input id="as-class" className="input" type="date" style={{ maxWidth: 200 }} value={classDay ?? (classOn ? isoToLocal(classOn).slice(0, 10) : "")} onChange={(e) => { setClassDay(e.target.value); setSaved(false); }} />
          <p className="hint">Nothing is submitted for this assignment, so it has no deadline.</p>
        </div>
      ) : (
        <>
          <div className="switchRow" style={{ display: "flex", justifyContent: "space-between", gap: 12, alignItems: "center" }}>
            <span style={{ fontSize: 14, fontWeight: 500 }}>Has a deadline</span>
            <Switch checked={hasDue} onChange={(v) => { setHasDue(v); setSaved(false); }} label="Has a deadline" />
          </div>
          {hasDue && (
            <>
              <div className="field">
                <label htmlFor="as-due">Deadline</label>
                <input id="as-due" className="input" type="datetime-local" value={due} onChange={(e) => { setDue(e.target.value); setSaved(false); }} aria-invalid={!!err("due_at")} />
                {err("due_at") && <p className="hint" style={{ color: "var(--danger)" }}>{err("due_at")}</p>}
              </div>
              <div className="field">
                <label htmlFor="as-dl">At the deadline</label>
                <select id="as-dl" className="select" value={onDeadline} onChange={(e) => { setOnDeadline(e.target.value); setSaved(false); }}>
                  <option value="keep_draft">Keep unsent drafts as drafts</option>
                  <option value="auto_submit">Submit unsent drafts for the student</option>
                </select>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between", gap: 12, alignItems: "center" }}>
                <span style={{ fontSize: 14, fontWeight: 500 }}>Accept late work</span>
                <Switch checked={late} onChange={(v) => { setLate(v); setSaved(false); }} label="Accept late work" />
              </div>
              {late && (
                <div className="field">
                  <label htmlFor="as-late">Late work until, optional</label>
                  <input id="as-late" className="input" type="datetime-local" value={lateUntil} onChange={(e) => { setLateUntil(e.target.value); setSaved(false); }} />
                </div>
              )}
            </>
          )}
          <div className="field">
            <label htmlFor="as-att">Attempts</label>
            <input id="as-att" className="input" style={{ maxWidth: 160 }} inputMode="numeric" placeholder="Unlimited" value={attempts} onChange={(e) => { setAttempts(e.target.value); setSaved(false); }} aria-invalid={!!err("max_attempts")} />
            <p className="hint">Leave empty to allow as many attempts as students need.</p>
            {err("max_attempts") && <p className="hint" style={{ color: "var(--danger)" }}>{err("max_attempts")}</p>}
          </div>
        </>
      )}
      {update.error && !update.error.errors.length && <p role="alert" className="alert alert-danger">{update.error.detail ?? update.error.title}</p>}
      <div className="row" style={{ gap: 12 }}>
        <button type="button" className="btn btn-primary" onClick={save} disabled={update.pending}>Save settings</button>
        <span role="status" className="muted" style={{ fontSize: 14 }}>{saved ? "Saved." : ""}</span>
      </div>
    </div>
  );
}
