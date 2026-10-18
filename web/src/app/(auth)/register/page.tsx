"use client";

import Link from "next/link";
import { useState, type FormEvent } from "react";
import { MockMailbox } from "@/components/auth/MockMailbox";
import { PasswordField } from "@/components/PasswordField";
import { StrengthMeter } from "@/components/StrengthMeter";
import { useRegister } from "@/lib/api/hooks/auth";
import { useT } from "@/lib/i18n";
import s from "../auth.module.css";

export default function RegisterPage() {
  const t = useT();
  const register = useRegister();
  const [name, setName] = useState("");
  const [surname, setSurname] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [sentTo, setSentTo] = useState<string | null>(null);
  const err = (f: string) => register.error?.errors.find((x) => x.field === f)?.message;

  async function submit(e: FormEvent) {
    e.preventDefault();
    const address = email.trim();
    if (await register.run({ name: name.trim(), surname: surname.trim(), email: address, password })) setSentTo(address);
  }

  if (sentTo) {
    return (
      <div className={s.card}>
        <div className={s.sent}>
          <h1 className={s.title}>{t("register.sentTitle")}</h1>
          <p className={s.lead} style={{ margin: 0 }}>{t("register.sentLead", { email: sentTo })}</p>
          <p className="hint">{t("register.sentHint")}</p>
          <Link href="/login" className="btn btn-outline">{t("auth.backToSignIn")}</Link>
        </div>
        <MockMailbox to={sentTo} />
      </div>
    );
  }

  return (
    <>
      <div className={s.card}>
        <h1 className={s.title}>{t("register.title")}</h1>
        <p className={s.lead}>{t("register.lead")}</p>
        <form className={s.form} onSubmit={submit} noValidate>
          <div className="field">
            <label htmlFor="name">{t("field.firstName")}</label>
            <input id="name" className="input" autoComplete="given-name" value={name} onChange={(e) => setName(e.target.value)} aria-invalid={!!err("name")} style={{ minHeight: 46 }} />
            {err("name") && <p className="field-error">{err("name")}</p>}
          </div>
          <div className="field">
            <label htmlFor="surname">{t("field.lastName")}</label>
            <input id="surname" className="input" autoComplete="family-name" value={surname} onChange={(e) => setSurname(e.target.value)} aria-invalid={!!err("surname")} style={{ minHeight: 46 }} />
            {err("surname") && <p className="field-error">{err("surname")}</p>}
          </div>
          <div className="field">
            <label htmlFor="email">{t("field.universityEmail")}</label>
            <input id="email" className="input" type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} aria-invalid={!!err("email")} style={{ minHeight: 46 }} />
            {err("email") && <p className="field-error">{err("email")}</p>}
          </div>
          <div>
            <PasswordField id="password" label={t("field.password")} value={password} onChange={setPassword} autoComplete="new-password" describedBy="strength" invalid={!!err("password")} error={err("password")} />
            <StrengthMeter password={password} id="strength" />
          </div>
          <p className="hint">{t("register.hint")}</p>
          <button type="submit" className="btn btn-primary btn-lg" disabled={register.pending}>{t("register.submit")}</button>
        </form>
      </div>
      <p className={s.foot}>
        {t("register.haveAccount")}{" "}
        <Link href="/login" className={s.link} style={{ fontWeight: 500 }}>{t("login.title")}</Link>
      </p>
    </>
  );
}
