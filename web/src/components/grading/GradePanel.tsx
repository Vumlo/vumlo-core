"use client";

import { useEffect, useState, type MutableRefObject } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { errorText } from "@/lib/api/client";
import { useGradeHistory, usePublishGrades, useSaveGrade } from "@/lib/api/hooks/assignments";
import type { GradeHistoryEntry, GradebookCell, GradebookColumn, GradebookRow } from "@/lib/api/types";
import { plural, sinceLabel } from "@/lib/format";
import { useT } from "@/lib/i18n";
import { validScore } from "@/lib/points";
import s from "./GradeWorkspace.module.css";

/** One recorded change as a line: the server sends the action and the values, the screen words them. */
function historyLine(h: GradeHistoryEntry, t: ReturnType<typeof useT>): string {
  const what =
    h.action === "publish"
      ? h.score ? t("gb.hPublished", { score: h.score }) : h.feedback ? t("gb.hPublishedFeedback") : t("gb.hWithdrawn")
      : h.score ? t("gb.hDraft", { score: h.score }) : h.feedback ? t("gb.hDraftFeedback") : t("gb.hCleared");
  return h.actor ? `${what}, ${h.actor.name} ${h.actor.surname}` : what;
}

/**
 * Points, comment and the buttons for one student. `dirty` tells the page whether the fields hold points or a comment
 * that were not saved; `drafts` is how many draft grades the assignment holds, for "publish them all".
 */
export function GradePanel({ assignmentId, row, col, cell, former, drafts, dirty, onSavedDraft }: {
  assignmentId: string;
  row: GradebookRow;
  col: GradebookColumn;
  cell: GradebookCell | undefined;
  former: boolean;
  drafts: number;
  dirty: MutableRefObject<boolean>;
  onSavedDraft: () => void;
}) {
  const t = useT();
  const save = useSaveGrade(assignmentId);
  const publish = usePublishGrades(assignmentId);
  const history = useGradeHistory(assignmentId, row.user_id);
  const [points, setPoints] = useState(cell?.draft_score ?? cell?.published_score ?? "");
  const [comment, setComment] = useState(cell?.draft_feedback ?? cell?.published_feedback ?? "");
  // What the server has: the fields differ from it until a save succeeds.
  const [stored, setStored] = useState({ points, comment });
  const [done, setDone] = useState<"draft" | "published" | null>(null);
  const [asking, setAsking] = useState(false);
  const [touched, setTouched] = useState(false);
  useEffect(() => {
    dirty.current = points !== stored.points || comment !== stored.comment;
    return () => { dirty.current = false; };
  }, [dirty, points, comment, stored]);
  const invalid = points !== "" && !validScore(points, col.max_points);

  async function saveDraft() {
    setTouched(true);
    if (invalid) return false;
    if (!(await save.run(row.user_id, points.trim() === "" ? null : points.trim(), comment))) return false;
    // The page moves on right after this; the effect above has not run yet, so clear the flag here.
    dirty.current = false;
    setStored({ points, comment });
    setDone("draft");
    return true;
  }
  async function saveAndNext() {
    if (await saveDraft()) onSavedDraft();
  }
  async function saveAndPublish() {
    if (!(await saveDraft())) return;
    if (await publish.run([row.user_id])) setDone("published");
  }
  const err = save.error ?? publish.error;

  return (
    <aside aria-labelledby="panel-h" className={`card ${s.panel}`}>
      <h3 id="panel-h" className={s.panelTitle}>{t("gw.panel")}</h3>
      <div className="stack" style={{ gap: "var(--space-3)" }}>
        <label htmlFor="points" className={s.label}>{t("cm.points")}</label>
        <div className="row" style={{ gap: "var(--space-3)" }}>
          <input id="points" className={`input ${s.points}`} inputMode="decimal" value={points} disabled={former} aria-invalid={touched && invalid} onChange={(e) => { setPoints(e.target.value); setTouched(false); setDone(null); }} />
          <span className="muted" style={{ fontSize: 14 }}>{t("gb.outOf", { max: col.max_points })}</span>
        </div>
        {touched && invalid && <p role="alert" className="hint" style={{ color: "var(--danger)" }}>{t("gb.invalid", { max: col.max_points })}</p>}
        <label htmlFor="comment" className={s.label}>{t("gb.commentFor", { name: row.name })}</label>
        <textarea id="comment" className={`textarea ${s.comment}`} value={comment} disabled={former} onChange={(e) => { setComment(e.target.value); setDone(null); }} />
        {err && <p role="alert" className="alert alert-danger">{errorText(err)}</p>}
        {done && <p role="status" className="alert alert-ok">{done === "draft" ? t("gb.savedDraft") : t("gb.publishedTo", { name: row.name })}</p>}
        {former ? <p className="muted" style={{ fontSize: 13 }}>{t("gb.left", { name: row.name })}</p> : (
          <>
            <div className={s.actions}>
              <button type="button" className="btn btn-primary" onClick={() => void saveAndNext()} disabled={save.pending}>{t("gw.saveNext")}</button>
              <button type="button" className="btn" onClick={() => void saveDraft()} disabled={save.pending}>{t("gb.saveDraft")}</button>
              <button type="button" className="btn" onClick={() => void saveAndPublish()} disabled={save.pending || publish.pending}>{t("cm.publish")}</button>
            </div>
            <p className="muted" style={{ fontSize: 13 }}>{t("gb.sees", { name: row.name })}</p>
          </>
        )}
      </div>
      <div className={s.rule}>
        <button type="button" className={`btn btn-quiet ${s.publishAll}`} disabled={drafts === 0 || publish.pending} onClick={() => setAsking(true)}>{t("grading.publishN", { n: plural(drafts, "draft") })}</button>
      </div>
      {asking && (
        <ConfirmDialog title={t("grading.confirmTitle", { title: col.title })} confirm={t("cm.publish")} pending={publish.pending} error={publish.error ? errorText(publish.error) : undefined} onClose={() => setAsking(false)} onConfirm={async () => { if (await publish.run("all")) setAsking(false); }}>
          {t("grading.confirmBody", { n: plural(drafts, "draft grade") })}
        </ConfirmDialog>
      )}
      <details className={s.rule}>
        <summary className={s.summary}>{t("gb.history")}</summary>
        <ol className={s.hist}>
          {(history.data ?? []).map((h) => <li key={h.id}><span>{historyLine(h, t)}</span><span className="muted">{sinceLabel(h.at)}</span></li>)}
          {history.data?.length === 0 && <li className="muted">{t("gb.noChanges")}</li>}
        </ol>
        <p className="muted" style={{ marginTop: "var(--space-2)", fontSize: 12 }}>{t("gb.recorded")}</p>
      </details>
    </aside>
  );
}
