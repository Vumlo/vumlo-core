"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState, type FormEvent } from "react";
import { Icon } from "@/components/Icon";
import { PasswordField } from "@/components/PasswordField";
import { useConsoleLogin } from "@/lib/api/hooks/console";
import { useT } from "@/lib/i18n";
import { rememberTemporaryPassword } from "@/lib/session";
import { consoleDemo } from "@/mocks/demoLogin";
import s from "../../../(auth)/auth.module.css";

/** Only a page of the console: the sign-in never sends anyone to the student app. */
const inConsole = (next: string | null) => (next && (next === "/console" || next.startsWith("/console/")) ? next : "/console");

function ConsoleLoginForm() {
  const t = useT();
  const router = useRouter();
  const next = inConsole(useSearchParams().get("next"));
  const login = useConsoleLogin();
  // Filled in only under `pnpm dev:mock`; a real build starts empty (see src/mocks/demoLogin.ts).
  const [email, setEmail] = useState(consoleDemo.email);
  const [password, setPassword] = useState(consoleDemo.password);

  async function signIn(e: FormEvent) {
    e.preventDefault();
    const user = await login.run(email.trim(), password);
    if (!user) return;
    if (user.must_change_password) rememberTemporaryPassword(password);
    router.replace(user.must_change_password ? `/console/change-password?next=${encodeURIComponent(next)}` : next);
  }

  const failed = !!login.error;
  return (
    <div className={s.card}>
      <h1 className={s.title}>{t("console.login.title")}</h1>
      <p className={s.lead}>{t("console.login.lead")}</p>
      {failed && (
        <div role="alert" className="alert alert-danger" style={{ marginBottom: 20 }}>
          <Icon name="info" size={18} style={{ flex: "none", marginTop: 0 }} />
          <span>{login.error?.status === 429 ? t("console.login.tooMany") : login.error?.status === 403 ? t("console.login.origin") : t("login.failed")}</span>
        </div>
      )}
      <form className={s.form} onSubmit={signIn}>
        <div className="field">
          <label htmlFor="email">{t("field.email")}</label>
          <input id="email" className="input" type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} style={{ minHeight: 46 }} />
        </div>
        <PasswordField id="password" label={t("field.password")} value={password} onChange={setPassword} autoComplete="current-password" invalid={failed} />
        <button type="submit" className="btn btn-primary btn-lg" disabled={login.pending} style={{ marginTop: 4 }}>
          {t("login.submit")}
        </button>
      </form>
    </div>
  );
}

export default function ConsoleLoginPage() {
  return (
    <Suspense fallback={null}>
      <ConsoleLoginForm />
    </Suspense>
  );
}
