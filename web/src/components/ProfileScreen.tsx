"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState, type FormEvent } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { PasswordField } from "@/components/PasswordField";
import { StrengthMeter } from "@/components/StrengthMeter";
import { useRevokeSessions, useSessions } from "@/lib/api/hooks/me";
import { useInstitution } from "@/lib/api/hooks/institution";
import { LANGUAGES, useLanguage, useT, type Language } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import { dayLong, initials, lastActive } from "@/lib/format";
import { fullName, useChangePassword, useLogout, useMe } from "@/lib/session";
import { useTheme, type ThemePreference } from "@/lib/theme";
import s from "./Profile.module.css";

const SECTIONS = [["profile", "profile.section.profile"], ["password", "profile.section.password"], ["sessions", "profile.section.sessions"], ["preferences", "profile.section.device"]] as const satisfies readonly (readonly [string, MessageKey])[];
const KIND: Record<string, { title: MessageKey; icon: IconName }> = {
  web: { title: "profile.kind.web", icon: "inClass" },
  app: { title: "profile.kind.app", icon: "form" },
  cli: { title: "profile.kind.cli", icon: "text" },
  pat: { title: "profile.kind.pat", icon: "lock" },
};
const ROLE = { student: "role.student", teacher: "role.teacher", admin: "role.admin" } as const;
const THEMES = { system: "theme.system", light: "theme.lightOption", dark: "theme.darkOption" } as const satisfies Record<ThemePreference, MessageKey>;

export function ProfileScreen() {
  const t = useT();
  const { language, setLanguage } = useLanguage();
  const router = useRouter();
  const me = useMe();
  const institution = useInstitution();
  const sessions = useSessions();
  const revoke = useRevokeSessions();
  const change = useChangePassword();
  const logout = useLogout();
  const { preference, setPreference } = useTheme();
  const [active, setActive] = useState<string>("profile");
  const [cur, setCur] = useState("");
  const [fresh, setFresh] = useState("");
  const [repeat, setRepeat] = useState("");
  const [done, setDone] = useState(false);
  const [resetDone, setResetDone] = useState(false);

  useEffect(() => {
    const els = SECTIONS.map(([id]) => document.getElementById(id)).filter((e): e is HTMLElement => !!e);
    const io = new IntersectionObserver((entries) => {
      const top = entries.filter((e) => e.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0];
      if (top) setActive(top.target.id);
    }, { rootMargin: "-90px 0px -60% 0px" });
    els.forEach((e) => io.observe(e));
    return () => io.disconnect();
  }, [me.data]);

  if (!me.data) return null;
  const u = me.data;
  const mismatch = repeat.length > 0 && fresh !== repeat;
  const fieldError = (f: string) => change.error?.errors.find((x) => x.field === f)?.message;
  const others = (sessions.data ?? []).filter((x) => !x.current);

  async function savePassword(e: FormEvent) {
    e.preventDefault();
    setDone(false);
    if (mismatch) return;
    if (await change.run(cur, fresh)) {
      setDone(true);
      setCur("");
      setFresh("");
      setRepeat("");
    }
  }

  return (
    <div className="container">
      <div className={s.layout}>
        <div className={s.nav}>
          <h1>{t("profile.title")}</h1>
          <nav aria-label={t("profile.sections")}>
            {SECTIONS.map(([id, label]) => (
              <a key={id} href={`#${id}`} className={s.link} aria-current={active === id ? "page" : undefined} onClick={() => setActive(id)}>{t(label)}</a>
            ))}
          </nav>
        </div>
        <div className={s.main}>
          <section id="profile" aria-labelledby="profile-h" className={`card ${s.section}`}>
            <h2 id="profile-h">{t("profile.section.profile")}</h2>
            <div className={s.who}>
              <span aria-hidden="true" className={s.big}>{initials(u.name, u.surname)}</span>
              <div>
                <p style={{ fontSize: 17, fontWeight: 600 }}>{fullName(u)}</p>
                <p className="muted" style={{ fontSize: 14 }}>{t("profile.roleAt", { role: t(ROLE[u.role]), institution: institution.data?.name ?? t("profile.yourInstitution") })}</p>
              </div>
            </div>
            <dl className={s.dl}>
              <dt>{t("field.name")}</dt><dd>{fullName(u)}</dd>
              <dt>{t("field.email")}</dt><dd>{u.email}</dd>
              <dt>{t("field.role")}</dt><dd>{t(ROLE[u.role])}</dd>
            </dl>
            <p className="alert alert-info" style={{ marginTop: 16 }}>
              <Icon name="info" size={18} style={{ flex: "none" }} />
              <span>{t("profile.managed")}</span>
            </p>
          </section>

          <section id="password" aria-labelledby="password-h" className={`card ${s.section}`}>
            <h2 id="password-h">{t("profile.section.password")}</h2>
            <p className={s.lead}>{t("profile.passwordLead")}</p>
            <form className={s.form} onSubmit={savePassword}>
              <PasswordField id="cur" label={t("profile.currentPassword")} value={cur} onChange={setCur} autoComplete="current-password" invalid={!!fieldError("current_password")} error={fieldError("current_password")} />
              <div>
                <PasswordField id="new" label={t("change.new")} value={fresh} onChange={setFresh} autoComplete="new-password" describedBy="strength" invalid={!!fieldError("new_password")} error={fieldError("new_password")} />
                <StrengthMeter password={fresh} id="strength" />
              </div>
              <PasswordField id="rep" label={t("change.repeat")} value={repeat} onChange={setRepeat} autoComplete="new-password" invalid={mismatch} error={mismatch ? t("change.mismatch") : undefined} />
              {done && <p role="status" className="alert alert-ok">{t("profile.passwordChanged")}</p>}
              <div><button type="submit" className="btn btn-primary" disabled={change.pending || !cur || !fresh || mismatch}>{t("profile.changePassword")}</button></div>
            </form>
          </section>

          <section id="sessions" aria-labelledby="sessions-h" className={`card ${s.section}`}>
            <div className="row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 12 }}>
              <h2 id="sessions-h">{t("profile.sessionsTitle")}</h2>
              <button type="button" className="btn btn-outline" disabled={others.length === 0 || revoke.pending} onClick={() => void revoke.run(others.map((x) => x.id))}>{t("profile.signOutOthers")}</button>
            </div>
            <p className={s.lead}>{t("profile.sessionsLead")}</p>
            <div>
              {(sessions.data ?? []).map((x) => {
                const k = KIND[x.kind] ?? KIND.web!;
                return (
                  <div key={x.id} className={s.sess}>
                    <span aria-hidden="true" className={s.sessIcon}><Icon name={k.icon} size={18} /></span>
                    <span className="grow">
                      <span style={{ display: "block", fontWeight: 500 }}>
                        {t(k.title)}{x.current && <span className="pill pill-accent" style={{ marginLeft: 8, padding: "0 8px", fontSize: 12 }}>{t("profile.thisBrowser")}</span>}
                      </span>
                      <span className="muted" style={{ display: "block", fontSize: 13 }}>{t("profile.sessionLine", { active: x.current ? t("date.activeNow") : lastActive(x.last_seen_at), since: dayLong(x.created_at) })}</span>
                    </span>
                    {!x.current && <button type="button" className="btn" onClick={() => void revoke.run([x.id])}>{t("account.signOut")}</button>}
                  </div>
                );
              })}
            </div>
          </section>

          <section id="preferences" aria-labelledby="pref-h" className={`card ${s.section}`}>
            <h2 id="pref-h">{t("profile.section.device")}</h2>
            <p className={s.lead}>{t("profile.deviceLead")}</p>
            <div className={s.pref}>
              <div><p style={{ fontWeight: 500 }}>{t("theme.label")}</p><p className="muted" style={{ fontSize: 13 }}>{t("theme.systemHint")}</p></div>
              <div role="radiogroup" aria-label={t("theme.label")} className="segmented">
                {(["system", "light", "dark"] as ThemePreference[]).map((p) => (
                  <button key={p} type="button" role="radio" aria-checked={preference === p} onClick={() => setPreference(p)}>{t(THEMES[p])}</button>
                ))}
              </div>
            </div>
            <div className={s.pref}>
              <label htmlFor="lang" style={{ fontWeight: 500 }}>{t("language.label")}</label>
              <select id="lang" className="select" style={{ width: "auto", minHeight: 40 }} value={language} onChange={(e) => setLanguage(e.target.value as Language)}>
                {LANGUAGES.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
              </select>
            </div>
            <div className={s.pref}>
              <div><p style={{ fontWeight: 500 }}>{t("profile.homeLayout")}</p><p className="muted" style={{ fontSize: 13 }}>{resetDone ? t("profile.homeReset") : t("profile.homeResetHint")}</p></div>
              <button type="button" className="btn btn-outline" onClick={() => { try { Object.keys(localStorage).filter((k) => k.startsWith("vumlo.home.")).forEach((k) => localStorage.removeItem(k)); } catch { /* storage blocked */ } setResetDone(true); }}>{t("profile.resetHome")}</button>
            </div>
          </section>

          <div>
            <button type="button" className="btn btn-outline" onClick={async () => { await logout.run(); router.replace("/login"); }}>
              <Icon name="logout" size={16} />
              {t("account.signOut")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
