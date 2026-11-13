import { useState, type FormEvent } from "react";
import { Icon } from "@/components/Icon";
import { PasswordField } from "@/components/PasswordField";
import { useConsoleLogin } from "@console/hooks/console";
import { navigate, useLocation } from "../router";
import { useT } from "@/lib/i18n";
import { rememberTemporaryPassword } from "@/lib/session";
import s from "@/app/(auth)/auth.module.css";

/** Only a page of the console: the sign-in leads nowhere else. */
const inConsole = (next: string | null) => (next && next.startsWith("/") && !next.startsWith("//") ? next : "/");

/** What the mock API fills in (`--mode mock` only: `src/mocks/start.ts` sets it), so a real build starts empty. */
const demo = (): { email: string; password: string } => (window as { __consoleDemo?: { email: string; password: string } }).__consoleDemo ?? { email: "", password: "" };

export function Login() {
  const t = useT();
  const next = inConsole(new URLSearchParams(useLocation().search).get("next"));
  const login = useConsoleLogin();
  const [email, setEmail] = useState(() => demo().email);
  const [password, setPassword] = useState(() => demo().password);

  async function signIn(e: FormEvent) {
    e.preventDefault();
    const user = await login.run(email.trim(), password);
    if (!user) return;
    if (user.must_change_password) rememberTemporaryPassword(password);
    navigate(user.must_change_password ? `/change-password?next=${encodeURIComponent(next)}` : next, true);
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
