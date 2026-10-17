"use client";

import Link from "next/link";
import { useState, type FormEvent } from "react";
import { MockMailbox } from "@/components/auth/MockMailbox";
import { useRequestPasswordReset } from "@/lib/api/hooks/auth";
import { useT } from "@/lib/i18n";
import s from "../auth.module.css";

export default function ForgotPasswordPage() {
  const t = useT();
  const request = useRequestPasswordReset();
  const [email, setEmail] = useState("");
  const [sentTo, setSentTo] = useState<string | null>(null);
  const err = request.error?.errors.find((x) => x.field === "email")?.message;

  async function submit(e: FormEvent) {
    e.preventDefault();
    const address = email.trim();
    if (await request.run(address)) setSentTo(address);
  }

  return (
    <div className={s.card}>
      {sentTo ? (
        <div className={s.sent}>
          <h1 className={s.title}>{t("forgot.sentTitle")}</h1>
          {/* The same words whether or not the address has an account, so the page does not reveal who is registered. */}
          <p className={s.lead} style={{ margin: 0 }}>{t("forgot.sentLead", { email: sentTo })}</p>
          <Link href="/login" className="btn btn-outline">{t("auth.backToSignIn")}</Link>
          <MockMailbox to={sentTo} />
        </div>
      ) : (
        <>
          <h1 className={s.title}>{t("forgot.title")}</h1>
          <p className={s.lead}>{t("forgot.lead")}</p>
          <form className={s.form} onSubmit={submit} noValidate>
            <div className="field">
              <label htmlFor="email">{t("field.email")}</label>
              <input id="email" className="input" type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} aria-invalid={!!err} style={{ minHeight: 46 }} />
              {err && <p className="field-error">{err}</p>}
            </div>
            <button type="submit" className="btn btn-primary btn-lg" disabled={request.pending || !email.trim()}>{t("forgot.submit")}</button>
          </form>
          <p className={s.signout}><Link href="/login" className={s.link}>{t("auth.backToSignIn")}</Link></p>
        </>
      )}
    </div>
  );
}
