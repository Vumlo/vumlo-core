"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState, type FormEvent } from "react";
import { Icon } from "@/components/Icon";
import { PasswordField } from "@/components/PasswordField";
import { StrengthMeter } from "@/components/StrengthMeter";
import { useInstitution } from "@/lib/api/hooks/institution";
import { api } from "@/lib/api/client";
import { clearQueries, invalidate } from "@/lib/query";
import { useChangePassword, useLogin, useLogout, type User } from "@/lib/session";
import { useTheme } from "@/lib/theme";
import s from "./login.module.css";

type Language = "en" | "uk";
const LANG_KEY = "learnly.language";

function readLanguage(): Language {
  try {
    return localStorage.getItem(LANG_KEY) === "uk" ? "uk" : "en";
  } catch {
    return "en";
  }
}

function LoginForm() {
  const router = useRouter();
  const params = useSearchParams();
  const next = params.get("next");
  const institution = useInstitution();
  const { theme, toggle } = useTheme();
  const login = useLogin();
  const logout = useLogout();
  const change = useChangePassword();
  const [email, setEmail] = useState("maya.okafor@northfield.edu");
  const [password, setPassword] = useState("correct horse");
  const [stage, setStage] = useState<"signin" | "change">("signin");
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const [fresh, setFresh] = useState("");
  const [repeat, setRepeat] = useState("");
  const [language, setLanguage] = useState<Language>(() => (typeof window === "undefined" ? "en" : readLanguage()));
  const name = institution.data?.name ?? "Learnly";

  function land(user: User) {
    const target = next && next.startsWith("/") ? next : user.role === "admin" ? "/admin" : "/";
    router.replace(target);
  }

  async function signIn(e: FormEvent) {
    e.preventDefault();
    setUnavailable(null);
    const user = await login.run(email.trim(), password);
    if (!user) return;
    if (user.must_change_password) {
      setStage("change");
      return;
    }
    land(user);
  }

  async function savePassword(e: FormEvent) {
    e.preventDefault();
    if (fresh !== repeat) return;
    if (!(await change.run(password, fresh))) return;
    const me = await api.get<User>("/me");
    invalidate("/me");
    land(me);
  }

  async function signOut() {
    await logout.run();
    clearQueries();
    setStage("signin");
    setPassword("");
  }

  function pickLanguage(l: Language) {
    setLanguage(l);
    try {
      localStorage.setItem(LANG_KEY, l);
    } catch {
      // Storage can be blocked; the choice is not kept.
    }
  }

  const failed = !!login.error;
  const mismatch = repeat.length > 0 && fresh !== repeat;
  const fieldError = (f: string) => change.error?.errors.find((x) => x.field === f)?.message;

  return (
    <div className={s.page}>
      <div className={s.center}>
        <div className={s.col}>
          <div className={s.brand}>
            <div aria-hidden="true" className={s.logo}>{name.charAt(0)}</div>
            <div>
              <div className={s.brandName}>{name}</div>
              <div className={s.brandSub}>Courses and grades</div>
            </div>
          </div>

          <div className={s.card}>
            {stage === "signin" ? (
              <>
                <h1 className={s.title}>Sign in</h1>
                <p className={s.lead}>Use your university email and password.</p>
                {failed && (
                  <div role="alert" className="alert alert-danger" style={{ marginBottom: 20 }}>
                    <Icon name="info" size={18} style={{ flex: "none", marginTop: 1 }} />
                    <span>That email and password don&apos;t match. Check both and try again.</span>
                  </div>
                )}
                {unavailable && (
                  <div role="status" className="alert alert-info" style={{ marginBottom: 20 }}>
                    <span>{unavailable}</span>
                  </div>
                )}
                <form className={s.form} onSubmit={signIn}>
                  <div className="field">
                    <label htmlFor="email">Email</label>
                    <input id="email" className="input" type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} style={{ minHeight: 46 }} />
                  </div>
                  <PasswordField
                    id="password"
                    label="Password"
                    value={password}
                    onChange={setPassword}
                    autoComplete="current-password"
                    invalid={failed}
                    trailing={
                      <a
                        href="#forgot"
                        className={s.link}
                        onClick={(e) => {
                          e.preventDefault();
                          setUnavailable("Password reset by email is not available on this server yet. Ask your administrator for a temporary password.");
                        }}
                      >
                        Forgot password?
                      </a>
                    }
                  />
                  <button type="submit" className="btn btn-primary btn-lg" disabled={login.pending} style={{ marginTop: 4 }}>
                    Sign in
                  </button>
                </form>
                <div className={s.or}>or</div>
                <button
                  type="button"
                  className="btn btn-outline btn-lg btn-block"
                  onClick={() => setUnavailable("Google sign-in is not available on this server yet.")}
                >
                  Continue with Google
                </button>
              </>
            ) : (
              <>
                <h1 className={s.title}>Choose a new password</h1>
                <p className={s.lead}>You signed in with a temporary password from your administrator. Choose your own to continue.</p>
                <form className={s.form} onSubmit={savePassword}>
                  <div>
                    <PasswordField
                      id="new-password"
                      label="New password"
                      value={fresh}
                      onChange={setFresh}
                      autoComplete="new-password"
                      describedBy="strength"
                      invalid={!!fieldError("new_password")}
                      error={fieldError("new_password")}
                    />
                    <StrengthMeter password={fresh} id="strength" />
                  </div>
                  <PasswordField
                    id="confirm-password"
                    label="Repeat new password"
                    value={repeat}
                    onChange={setRepeat}
                    autoComplete="new-password"
                    invalid={mismatch}
                    error={mismatch ? "The two passwords are different" : undefined}
                  />
                  <p className="hint">A few unrelated words make a password that is long and easy to remember. Don&apos;t reuse one from another site.</p>
                  <button type="submit" className="btn btn-primary btn-lg" disabled={change.pending || !fresh || mismatch}>
                    Save and continue
                  </button>
                </form>
                <p className={s.signout}>
                  <a href="#signout" className={s.link} onClick={(e) => { e.preventDefault(); void signOut(); }}>Sign out</a>
                </p>
              </>
            )}
          </div>

          {stage === "signin" && (
            <p className={s.foot}>
              New here?{" "}
              <Link
                href="#register"
                className={s.link}
                style={{ fontWeight: 500 }}
                onClick={(e) => {
                  e.preventDefault();
                  setUnavailable("Accounts are created by your administrator for now. Self-registration is not available on this server yet.");
                }}
              >
                Create an account
              </Link>{" "}
              with your university email.
            </p>
          )}
        </div>
      </div>

      <footer className={s.footer}>
        <label className={s.langLabel}>
          <Icon name="globe" size={18} />
          <span className="sr-only">Language</span>
          <select value={language} onChange={(e) => pickLanguage(e.target.value as Language)}>
            <option value="en">English</option>
            <option value="uk">Українська</option>
          </select>
        </label>
        <button type="button" className={s.themeBtn} onClick={toggle}>
          <Icon name={theme === "dark" ? "sun" : "moon"} size={18} />
          {theme === "dark" ? "Light theme" : "Dark theme"}
        </button>
      </footer>
    </div>
  );
}

export default function LoginPage() {
  return (
    <Suspense fallback={null}>
      <LoginForm />
    </Suspense>
  );
}
