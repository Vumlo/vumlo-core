"use client";

import { useState } from "react";

import { ApiError, errorText } from "@/lib/api/client";
import { dateTime } from "@/lib/format";
import { Icon } from "@/components/Icon";
import { useT } from "@/lib/i18n";
import { invalidate, useMutation } from "@/lib/query";
import { CHAT, confirmProposal, rejectProposal, type Proposal } from "./api";
import s from "./Buddy.module.css";

export type Settled = "done" | "dismissed";

type T = ReturnType<typeof useT>;

const KNOWN = ["set_grade", "publish_grades", "set_deadline", "create_module", "create_text_item", "create_assignment", "create_quiz", "save_draft", "submit_work", "create_term", "create_group", "add_group_members", "reset_password"];
const CREATES = ["create_module", "create_text_item", "create_assignment", "create_quiz"];
/** The cards that show a text the person is about to save or hand in. */
const OWN_WORK = ["save_draft", "submit_work"];

/** What the change will do, worded in the person's language from the facts the server read for them. */
function sentence(p: Proposal, t: T): string {
  const f = p.facts;
  const assignment = f.assignment ?? "";
  switch (p.tool) {
    case "set_grade":
      return f.score
        ? t("buddy.p.setGrade", { student: f.student ?? "", assignment, score: f.score, max: f.max_points ?? "" })
        : t("buddy.p.clearGrade", { student: f.student ?? "", assignment });
    case "publish_grades":
      return t("buddy.p.publish", { assignment });
    case "set_deadline":
      return f.due_at ? t("buddy.p.setDeadline", { assignment, due: dateTime(f.due_at) }) : t("buddy.p.clearDeadline", { assignment });
    case "create_module":
      return t("buddy.p.createModule", { title: f.title ?? "", course: f.course ?? "" });
    case "create_text_item":
      return t("buddy.p.createText", { title: f.title ?? "", module: f.module ?? "", course: f.course ?? "" });
    case "create_assignment":
      return t("buddy.p.createAssignment", { title: f.title ?? "", points: f.max_points ?? "", module: f.module ?? "", course: f.course ?? "" });
    case "create_quiz":
      return t("buddy.p.createQuiz", { title: f.title ?? "", questions: f.questions ?? "", points: f.max_points ?? "", module: f.module ?? "", course: f.course ?? "" });
    case "save_draft":
      return t("buddy.p.saveDraft", { assignment });
    case "submit_work":
      return t("buddy.p.submitWork", { assignment });
    case "create_term":
      return t("buddy.p.createTerm", { name: f.name ?? "", from: f.starts_on ?? "", to: f.ends_on ?? "" });
    case "create_group":
      return t("buddy.p.createGroup", { name: f.name ?? "" });
    case "add_group_members":
      return t("buddy.p.addMembers", { count: f.count ?? "", group: f.group ?? "" });
    case "reset_password":
      return t("buddy.p.resetPassword", { person: f.person ?? "", email: f.email ?? "" });
    default:
      return t("buddy.p.unknown", { tool: p.tool });
  }
}

/** A refusal from the server is final (the change was not made and is not offered again); a lost connection is not. */
const final = (e: ApiError | undefined) => e !== undefined && e.status >= 400 && e.status < 500;

/** A change the Buddy prepared, with the two buttons that decide it. Nothing happens before "Confirm". */
export function ProposalCard({ proposal, settled, onSettled }: { proposal: Proposal; settled?: Settled; onSettled: (s: Settled) => void }) {
  const t = useT();
  // A temporary password lives in this card only, in memory: it is not in the chat, and it is gone when the panel closes.
  const [password, setPassword] = useState<string | null>(null);
  const confirm = useMutation(async () => {
    const done = await confirmProposal(proposal.id);
    setPassword(done?.temporary_password ?? null);
    // What the change touched is on other screens too: have them ask again.
    invalidate("/");
    onSettled("done");
  });
  const dismiss = useMutation(async () => {
    await rejectProposal(proposal.id);
    invalidate(CHAT);
    onSettled("dismissed");
  });
  const busy = confirm.pending || dismiss.pending;
  const refused = final(confirm.error) || final(dismiss.error);
  const error = confirm.error ?? dismiss.error;

  if (settled === "done" && password) {
    const person = proposal.facts.person ?? "";
    return (
      <div className={s.proposal} data-card data-state="done">
        <p className={s.proposalText}>{sentence(proposal, t)}</p>
        <p className={s.proposalNote}>{t("ap.giveTo", { name: person })}</p>
        <code className={s.password} aria-label={t("buddy.p.password", { person })}>
          {password}
        </code>
        <div className={s.proposalActions}>
          <button type="button" className="btn" onClick={() => void navigator.clipboard?.writeText(password)}>
            <Icon name="copy" size={15} />
            {t("be.copy")}
          </button>
          <button type="button" className="btn btn-quiet" onClick={() => setPassword(null)}>
            {t("common.done")}
          </button>
        </div>
      </div>
    );
  }
  if (settled) {
    return (
      <p className={s.settled} data-state={settled} role="status">
        {sentence(proposal, t)} {settled === "done" ? t("buddy.confirmed") : t("buddy.dismissed")}
      </p>
    );
  }
  return (
    <div className={s.proposal} data-card>
      <p className={s.proposalText}>{sentence(proposal, t)}</p>
      {proposal.tool === "set_grade" && proposal.facts.feedback && <p className={s.proposalNote}>{t("buddy.p.feedback", { feedback: proposal.facts.feedback })}</p>}
      {proposal.tool === "set_grade" && <p className={s.proposalNote}>{t("buddy.p.draftNote")}</p>}
      {OWN_WORK.includes(proposal.tool) && proposal.facts.preview && (
        <pre className={s.preview} tabIndex={0} aria-label={t("buddy.p.preview")}>
          {proposal.facts.preview}
        </pre>
      )}
      {proposal.tool === "save_draft" && <p className={s.proposalNote}>{t("buddy.p.ownWords")}</p>}
      {proposal.tool === "submit_work" && <p className={s.proposalNote}>{t("buddy.p.handedIn")}</p>}
      {CREATES.includes(proposal.tool) && proposal.facts.preview && (
        <pre className={s.preview} tabIndex={0} aria-label={t("buddy.p.preview")}>
          {proposal.facts.preview}
        </pre>
      )}
      {CREATES.includes(proposal.tool) && <p className={s.proposalNote}>{t("buddy.p.draftOnly")}</p>}
      {proposal.tool === "create_quiz" && <p className={s.proposalNote}>{t("buddy.p.checkQuiz")}</p>}
      {proposal.tool === "add_group_members" && proposal.facts.people && <p className={s.proposalNote}>{proposal.facts.people}</p>}
      {proposal.tool === "reset_password" && <p className={s.proposalNote}>{t("buddy.p.resetNote")}</p>}
      {error && (
        <p className={s.error} role="alert">
          {t("buddy.notDone", { reason: errorText(error) ?? "" })}
        </p>
      )}
      <div className={s.proposalActions}>
        {!refused && KNOWN.includes(proposal.tool) && (
          <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void confirm.run()}>
            {t("buddy.confirm")}
          </button>
        )}
        <button type="button" className="btn btn-quiet" disabled={busy} onClick={() => void dismiss.run()}>
          {t("buddy.dismiss")}
        </button>
      </div>
    </div>
  );
}
