"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useState, type FormEvent } from "react";
import { PasswordField } from "@/components/PasswordField";
import { StrengthMeter } from "@/components/StrengthMeter";
import { useConfirmPasswordReset } from "@/lib/api/hooks/auth";
import { useT } from "@/lib/i18n";
import s from "../auth.module.css";

/** The page behind the link in the reset mail (ADR-0017). A reset ends every session of the account. */
function Reset() {
  const t = useT();
  const token = useSearchParams().get("token") ?? "";
  const confirm = useConfirmPasswordReset();
  const [fresh, setFresh] = useState("");
  const [repeat, setRepeat] = useState("");
  const [done, setDone] = useState(false);
  const mismatch = repeat.length > 0 && fresh !== repeat;
  const expired = !token || confirm.error?.status === 410;
  const fieldError = confirm.error?.errors.find((x) => x.field === "new_password")?.message;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (fresh === repeat && (await confirm.run(token, fresh))) setDone(true);
  }

  if (done) {
    return (
      <div className={s.card}>
        <div className={s.sent}>
          <h1 className={s.title}>{t("reset.doneTitle")}</h1>
          <p className={s.lead} style={{ margin: 0 }}>{t("reset.doneLead")}</p>
          <Link href="/login" className="btn btn-primary btn-lg">{t("login.submit")}</Link>
        </div>
      </div>
    );
  }
  if (expired) {
    return (
      <div className={s.card}>
        <div className={s.sent}>
          <h1 className={s.title}>{t("reset.expiredTitle")}</h1>
          <p role="alert" className={s.lead} style={{ margin: 0 }}>{t("reset.expiredLead")}</p>
          <Link href="/forgot-password" className="btn btn-outline">{t("reset.askAgain")}</Link>
        </div>
      </div>
    );
  }
  return (
    <div className={s.card}>
      <h1 className={s.title}>{t("reset.title")}</h1>
      <p className={s.lead}>{t("reset.lead")}</p>
      <form className={s.form} onSubmit={submit}>
        <div>
          <PasswordField id="new-password" label={t("change.new")} value={fresh} onChange={setFresh} autoComplete="new-password" describedBy="strength" invalid={!!fieldError} error={fieldError} />
          <StrengthMeter password={fresh} id="strength" />
        </div>
        <PasswordField id="confirm-password" label={t("change.repeat")} value={repeat} onChange={setRepeat} autoComplete="new-password" invalid={mismatch} error={mismatch ? t("change.mismatch") : undefined} />
        <p className="hint">{t("change.hint")}</p>
        <button type="submit" className="btn btn-primary btn-lg" disabled={confirm.pending || !fresh || mismatch}>{t("reset.submit")}</button>
      </form>
    </div>
  );
}

export default function ResetPasswordPage() {
  return (
    <Suspense fallback={null}>
      <Reset />
    </Suspense>
  );
}
