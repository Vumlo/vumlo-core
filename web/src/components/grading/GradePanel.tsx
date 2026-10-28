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
export function GradePanel({ assignmentId, row, col, cell, former, drafts, dirty, onSaved }: {
  assignmentId: string;
  row: GradebookRow;
  col: GradebookColumn;
  cell: GradebookCell | undefined;
  former: boolean;
  drafts: number;
  dirty: MutableRefObject<boolean>;
  /** Tells the page what was saved; `advance` asks it to move on to the next student, `before` is what the server held before a draft was saved (so it can be put back). */
  onSaved: (kind: "draft" | "published", advance: boolean, before?: { points: string; comment: string }) => void;
}) {
  const t = useT();
  const save = useSaveGrade(assignmentId);
  const publish = usePublishGrades(assignmentId);
  const history = useGradeHistory(assignmentId, row.user_id);
  const [points, setPoints] = useState(cell?.draft_score ?? cell?.published_score ?? "");
  const [comment, setComment] = useState(cell?.draft_feedback ?? cell?.published_feedback ?? "");
  // What the server has: the fields differ from it until a save succeeds.
  const [stored, setStored] = useState({ points, comment });
  const [asking, setAsking] = useState(false);
  const [touched, setTouched] = useState(false);
  useEffect(() => {
    dirty.current = points !== stored.points || comment !== stored.comment;
    return () => { dirty.current = false; };
  }, [dirty, points, comment, stored]);
  const invalid = points !== "" && !validScore(points, col.max_points);

  async function saveDraft() {
    setTouched(true);
    if (invalid) {
      document.getElementById("points")?.focus();
      return false;
    }
    if (!(await save.run(row.user_id, points.trim() === "" ? null : points.trim(), comment))) return false;
    // The page moves on right after this; the effect above has not run yet, so clear the flag here.
    dirty.current = false;
    setStored({ points, comment });
    return true;
  }
  async function saveOnly() {
    const before = stored;
    if (await saveDraft()) onSaved("draft", false, before);
  }
  async function saveAndNext() {
    const before = stored;
    if (await saveDraft()) onSaved("draft", true, before);
  }
  async function saveAndPublish() {
    if (!(await saveDraft())) return;
    if (await publish.run([row.user_id])) onSaved("published", true);
  }
  // Publishing nothing would hide a grade that was never given; it is for a grade or a comment, or to withdraw one.
  const blank = points.trim() === "" && comment.trim() === "" && !cell?.published_score && !cell?.published_feedback;
  const err = save.error ?? publish.error;

  return (
    <aside aria-labelledby="panel-h" className={`card ${s.panel}`}>
      <h3 id="panel-h" className={s.panelTitle}>{t("gw.panel")}</h3>
      <form className="stack" style={{ gap: "var(--space-3)" }} onSubmit={(e) => { e.preventDefault(); void saveAndNext(); }}>
        <label htmlFor="points" className={s.label}>{t("cm.points")}</label>
        <div className="row" style={{ gap: "var(--space-3)" }}>
          <input id="points" className={`input ${s.points}`} inputMode="decimal" value={points} disabled={former} aria-invalid={touched && invalid} aria-describedby="student-h" onChange={(e) => { setPoints(e.target.value); setTouched(false); }} />
          <span className="muted" style={{ fontSize: 14 }}>{t("gb.outOf", { max: col.max_points })}</span>
        </div>
        <label htmlFor="comment" className={s.label}>{t("gb.commentFor", { name: row.name })}</label>
        <textarea id="comment" className={`textarea ${s.comment}`} value={comment} disabled={former} onChange={(e) => { setComment(e.target.value); }} />
        {err && <p role="alert" className="alert alert-danger">{errorText(err)}</p>}
        {former ? <p className="muted" style={{ fontSize: 13 }}>{t("gb.left", { name: row.name })}</p> : (
          <>
            <div className={s.actions}>
              <button type="submit" className="btn btn-primary" disabled={save.pending}>{t("gw.saveNext")}</button>
              <button type="button" className="btn" onClick={() => void saveOnly()} disabled={save.pending}>{t("gb.saveDraft")}</button>
              <button type="button" className="btn" onClick={() => void saveAndPublish()} disabled={blank || save.pending || publish.pending}>{t("cm.publish")}</button>
            </div>
            {touched && invalid && <p role="alert" className="hint" style={{ color: "var(--danger)" }}>{t("gb.invalid", { max: col.max_points })}</p>}
            <p className="muted" style={{ fontSize: 13 }}>{t("gb.sees", { name: row.name })}</p>
          </>
        )}
      </form>
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
