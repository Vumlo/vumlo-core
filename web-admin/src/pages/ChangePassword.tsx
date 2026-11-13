import { useEffect, useState, type FormEvent } from "react";
import { PasswordField } from "@/components/PasswordField";
import { StrengthMeter } from "@/components/StrengthMeter";
import { clearQueries } from "@/lib/query";
import { useT } from "@/lib/i18n";
import { temporaryPassword, useChangePassword, useLogout, useMe } from "@/lib/session";
import s from "@/app/(auth)/auth.module.css";
import { navigate, useLocation } from "../router";

/** Where an administrator with a temporary password lands: until they choose their own, nothing else opens (ADR-0017). */
export function ChangePassword() {
  const t = useT();
  const next = new URLSearchParams(useLocation().search).get("next");
  const me = useMe();
  const change = useChangePassword();
  const logout = useLogout();
  const [current, setCurrent] = useState(temporaryPassword);
  const [fresh, setFresh] = useState("");
  const [repeat, setRepeat] = useState("");
  const user = me.data;
  // Only a page of the console.
  const target = next && next.startsWith("/") && !next.startsWith("//") ? next : "/";

  useEffect(() => {
    if (me.error?.status === 401) navigate("/login", true);
    else if (user && !user.must_change_password) navigate(target, true);
  }, [me.error, user, target]);

  if (!user?.must_change_password) return null;

  async function save(e: FormEvent) {
    e.preventDefault();
    if (fresh !== repeat) return;
    if (await change.run(current, fresh)) navigate(target, true);
  }

  async function signOut() {
    await logout.run();
    clearQueries();
    navigate("/login", true);
  }

  const mismatch = repeat.length > 0 && fresh !== repeat;
  const fieldError = (f: string) => change.error?.errors.find((x) => x.field === f)?.message;
  return (
    <div className={s.card}>
      <h1 className={s.title}>{t("change.title")}</h1>
      <p className={s.lead}>{t("change.lead")}</p>
      <form className={s.form} onSubmit={save}>
        <PasswordField id="current-password" label={t("change.current")} value={current} onChange={setCurrent} autoComplete="current-password" invalid={!!fieldError("current_password")} error={fieldError("current_password")} />
        <div>
          <PasswordField id="new-password" label={t("change.new")} value={fresh} onChange={setFresh} autoComplete="new-password" describedBy="strength" invalid={!!fieldError("new_password")} error={fieldError("new_password")} />
          <StrengthMeter password={fresh} id="strength" />
        </div>
        <PasswordField id="confirm-password" label={t("change.repeat")} value={repeat} onChange={setRepeat} autoComplete="new-password" invalid={mismatch} error={mismatch ? t("change.mismatch") : undefined} />
        <p className="hint">{t("change.hint")}</p>
        <button type="submit" className="btn btn-primary btn-lg" disabled={change.pending || !current || !fresh || mismatch}>
          {t("change.submit")}
        </button>
      </form>
      <p className={s.signout}>
        <button type="button" className={s.link} onClick={() => void signOut()}>{t("account.signOut")}</button>
      </p>
    </div>
  );
}
