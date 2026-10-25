"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useState, type FormEvent } from "react";
import { PasswordField } from "@/components/PasswordField";
import { StrengthMeter } from "@/components/StrengthMeter";
import { useVerifyEmail } from "@/lib/api/hooks/auth";
import { useT } from "@/lib/i18n";
import s from "../auth.module.css";

/**
 * The page behind the link in the verification mail. The link proves the mailbox, and whoever holds it
 * chooses the password of the account (ADR-0067). The token is single use, and it is spent only once the
 * password is accepted.
 */
function Verify() {
  const t = useT();
  const token = useSearchParams().get("token") ?? "";
  const verify = useVerifyEmail();
  const [fresh, setFresh] = useState("");
  const [repeat, setRepeat] = useState("");
  const [done, setDone] = useState(false);
  const mismatch = repeat.length > 0 && fresh !== repeat;
  const gone = !token || verify.error?.status === 410;
  const fieldError = verify.error?.errors.find((x) => x.field === "password")?.message;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (fresh === repeat && (await verify.run(token, fresh))) setDone(true);
  }

  if (done) {
    return (
      <div className={s.card}>
        <div className={s.sent}>
          <h1 className={s.title}>{t("verify.doneTitle")}</h1>
          <p className={s.lead} style={{ margin: 0 }}>{t("verify.doneLead")}</p>
          <Link href="/login" className="btn btn-primary btn-lg">{t("login.submit")}</Link>
        </div>
      </div>
    );
  }
  if (gone) {
    return (
      <div className={s.card}>
        <div className={s.sent}>
          <h1 className={s.title}>{t("verify.failedTitle")}</h1>
          <p role="alert" className={s.lead} style={{ margin: 0 }}>{t("verify.failedLead")}</p>
          <Link href="/register" className="btn btn-outline">{t("verify.again")}</Link>
        </div>
      </div>
    );
  }
  return (
    <div className={s.card}>
      <h1 className={s.title}>{t("verify.title")}</h1>
      <p className={s.lead}>{t("verify.lead")}</p>
      <form className={s.form} onSubmit={submit}>
        <div>
          <PasswordField id="new-password" label={t("change.new")} value={fresh} onChange={setFresh} autoComplete="new-password" describedBy="strength" invalid={!!fieldError} error={fieldError} />
          <StrengthMeter password={fresh} id="strength" />
        </div>
        <PasswordField id="confirm-password" label={t("change.repeat")} value={repeat} onChange={setRepeat} autoComplete="new-password" invalid={mismatch} error={mismatch ? t("change.mismatch") : undefined} />
        <p className="hint">{t("change.hint")}</p>
        <button type="submit" className="btn btn-primary btn-lg" disabled={verify.pending || !fresh || mismatch}>{t("verify.submit")}</button>
      </form>
    </div>
  );
}

export default function VerifyEmailPage() {
  return (
    <Suspense fallback={null}>
      <Verify />
    </Suspense>
  );
}
