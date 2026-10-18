"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState, type FormEvent } from "react";
import { Icon } from "@/components/Icon";
import { PasswordField } from "@/components/PasswordField";
import { useGoogleStart } from "@/lib/api/hooks/auth";
import { useT } from "@/lib/i18n";
import { landingPath, rememberTemporaryPassword, useLogin } from "@/lib/session";
import s from "../auth.module.css";

const mocked = process.env.NEXT_PUBLIC_API_MOCKING === "enabled";

function LoginForm() {
  const t = useT();
  const router = useRouter();
  const params = useSearchParams();
  const next = params.get("next");
  const login = useLogin();
  const google = useGoogleStart();
  const [email, setEmail] = useState(mocked ? "maya.okafor@northfield.edu" : "");
  const [password, setPassword] = useState(mocked ? "correct horse" : "");

  async function signIn(e: FormEvent) {
    e.preventDefault();
    const user = await login.run(email.trim(), password);
    if (!user) return;
    if (user.must_change_password) rememberTemporaryPassword(password);
    router.replace(landingPath(user, next));
  }

  async function withGoogle() {
    const r = await google.run(email.trim());
    if (r) window.location.assign(r.authorization_url);
  }

  const failed = !!login.error;
  return (
    <>
      <div className={s.card}>
        <h1 className={s.title}>{t("login.title")}</h1>
        <p className={s.lead}>{t("login.lead")}</p>
        {failed && (
          <div role="alert" className="alert alert-danger" style={{ marginBottom: 20 }}>
            <Icon name="info" size={18} style={{ flex: "none", marginTop: 1 }} />
            {/* 403 after the form was sent: the password was right but the address is not verified (ADR-0046). */}
            <span>{login.error?.status === 403 ? t("login.notVerified") : t("login.failed")}</span>
          </div>
        )}
        {google.error && (
          <div role="alert" className="alert alert-danger" style={{ marginBottom: 20 }}>
            <span>{t("login.googleFailed")}</span>
          </div>
        )}
        <form className={s.form} onSubmit={signIn}>
          <div className="field">
            <label htmlFor="email">{t("field.email")}</label>
            <input id="email" className="input" type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} style={{ minHeight: 46 }} />
          </div>
          <PasswordField
            id="password"
            label={t("field.password")}
            value={password}
            onChange={setPassword}
            autoComplete="current-password"
            invalid={failed}
            trailing={<Link href="/forgot-password" className={s.link}>{t("login.forgot")}</Link>}
          />
          <button type="submit" className="btn btn-primary btn-lg" disabled={login.pending} style={{ marginTop: 4 }}>
            {t("login.submit")}
          </button>
        </form>
        <div className={s.or}>{t("login.or")}</div>
        <button type="button" className="btn btn-outline btn-lg btn-block" disabled={google.pending} onClick={withGoogle}>
          {t("login.google")}
        </button>
      </div>
      <p className={s.foot}>
        {t("login.newHere")}{" "}
        <Link href="/register" className={s.link} style={{ fontWeight: 500 }}>{t("login.create")}</Link>{" "}
        {t("login.withUniversityEmail")}
      </p>
    </>
  );
}

export default function LoginPage() {
  return (
    <Suspense fallback={null}>
      <LoginForm />
    </Suspense>
  );
}
