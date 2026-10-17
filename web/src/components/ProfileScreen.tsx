"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState, type FormEvent } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { PasswordField } from "@/components/PasswordField";
import { StrengthMeter } from "@/components/StrengthMeter";
import { useRevokeSessions, useSessions } from "@/lib/api/hooks/me";
import { useInstitution } from "@/lib/api/hooks/institution";
import { dayLong, initials, lastActive } from "@/lib/format";
import { fullName, useChangePassword, useLogout, useMe } from "@/lib/session";
import { useTheme, type ThemePreference } from "@/lib/theme";
import s from "./Profile.module.css";

const SECTIONS = [["profile", "Profile"], ["password", "Password"], ["sessions", "Sessions"], ["preferences", "This device"]] as const;
const KIND: Record<string, { title: string; icon: IconName }> = {
  web: { title: "Web browser", icon: "inClass" },
  app: { title: "Mobile app", icon: "form" },
  cli: { title: "Command line", icon: "text" },
  pat: { title: "Access token", icon: "lock" },
};
const ROLE = { student: "Student", teacher: "Teacher", admin: "Administrator" } as const;
const LANG_KEY = "learnly.language";

export function ProfileScreen() {
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
  const [language, setLanguage] = useState(() => { try { return localStorage.getItem(LANG_KEY) === "uk" ? "uk" : "en"; } catch { return "en"; } });
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
          <h1>Settings</h1>
          <nav aria-label="Settings sections">
            {SECTIONS.map(([id, label]) => (
              <a key={id} href={`#${id}`} className={s.link} aria-current={active === id ? "page" : undefined} onClick={() => setActive(id)}>{label}</a>
            ))}
          </nav>
        </div>
        <div className={s.main}>
          <section id="profile" aria-labelledby="profile-h" className={`card ${s.section}`}>
            <h2 id="profile-h">Profile</h2>
            <div className={s.who}>
              <span aria-hidden="true" className={s.big}>{initials(u.name, u.surname)}</span>
              <div>
                <p style={{ fontSize: 17, fontWeight: 600 }}>{fullName(u)}</p>
                <p className="muted" style={{ fontSize: 14 }}>{ROLE[u.role]} at {institution.data?.name ?? "your institution"}</p>
              </div>
            </div>
            <dl className={s.dl}>
              <dt>Name</dt><dd>{fullName(u)}</dd>
              <dt>Email</dt><dd>{u.email}</dd>
              <dt>Role</dt><dd>{ROLE[u.role]}</dd>
            </dl>
            <p className="alert alert-info" style={{ marginTop: 16 }}>
              <Icon name="info" size={18} style={{ flex: "none" }} />
              <span>Your name, email and role come from your institution. To change them, contact your administrator.</span>
            </p>
          </section>

          <section id="password" aria-labelledby="password-h" className={`card ${s.section}`}>
            <h2 id="password-h">Password</h2>
            <p className={s.lead}>Changing your password signs you out on every other device.</p>
            <form className={s.form} onSubmit={savePassword}>
              <PasswordField id="cur" label="Current password" value={cur} onChange={setCur} autoComplete="current-password" invalid={!!fieldError("current_password")} error={fieldError("current_password")} />
              <div>
                <PasswordField id="new" label="New password" value={fresh} onChange={setFresh} autoComplete="new-password" describedBy="strength" invalid={!!fieldError("new_password")} error={fieldError("new_password")} />
                <StrengthMeter password={fresh} id="strength" />
              </div>
              <PasswordField id="rep" label="Repeat new password" value={repeat} onChange={setRepeat} autoComplete="new-password" invalid={mismatch} error={mismatch ? "The two passwords are different" : undefined} />
              {done && <p role="status" className="alert alert-ok">Your password is changed.</p>}
              <div><button type="submit" className="btn btn-primary" disabled={change.pending || !cur || !fresh || mismatch}>Change password</button></div>
            </form>
          </section>

          <section id="sessions" aria-labelledby="sessions-h" className={`card ${s.section}`}>
            <div className="row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 12 }}>
              <h2 id="sessions-h">Where you&apos;re signed in</h2>
              <button type="button" className="btn btn-outline" disabled={others.length === 0 || revoke.pending} onClick={() => void revoke.run(others.map((x) => x.id))}>Sign out everywhere else</button>
            </div>
            <p className={s.lead}>If you don&apos;t recognise a session, sign it out and change your password.</p>
            <div>
              {(sessions.data ?? []).map((x) => {
                const k = KIND[x.kind] ?? KIND.web!;
                return (
                  <div key={x.id} className={s.sess}>
                    <span aria-hidden="true" className={s.sessIcon}><Icon name={k.icon} size={18} /></span>
                    <span className="grow">
                      <span style={{ display: "block", fontWeight: 500 }}>
                        {k.title}{x.current && <span className="pill pill-accent" style={{ marginLeft: 8, padding: "0 8px", fontSize: 12 }}>This browser</span>}
                      </span>
                      <span className="muted" style={{ display: "block", fontSize: 13 }}>{x.current ? "Active now" : lastActive(x.last_seen_at)}, signed in {dayLong(x.created_at)}</span>
                    </span>
                    {!x.current && <button type="button" className="btn" onClick={() => void revoke.run([x.id])}>Sign out</button>}
                  </div>
                );
              })}
            </div>
          </section>

          <section id="preferences" aria-labelledby="pref-h" className={`card ${s.section}`}>
            <h2 id="pref-h">This device</h2>
            <p className={s.lead}>These settings are saved in this browser only.</p>
            <div className={s.pref}>
              <div><p style={{ fontWeight: 500 }}>Theme</p><p className="muted" style={{ fontSize: 13 }}>System follows your computer&apos;s setting.</p></div>
              <div role="radiogroup" aria-label="Theme" className="segmented">
                {(["system", "light", "dark"] as ThemePreference[]).map((t) => (
                  <button key={t} type="button" role="radio" aria-checked={preference === t} onClick={() => setPreference(t)}>{t === "system" ? "System" : t === "light" ? "Light" : "Dark"}</button>
                ))}
              </div>
            </div>
            <div className={s.pref}>
              <label htmlFor="lang" style={{ fontWeight: 500 }}>Language</label>
              <select id="lang" className="select" style={{ width: "auto", minHeight: 40 }} value={language} onChange={(e) => { setLanguage(e.target.value); try { localStorage.setItem(LANG_KEY, e.target.value); } catch { /* storage blocked */ } }}>
                <option value="en">English</option>
                <option value="uk">Українська</option>
              </select>
            </div>
            <div className={s.pref}>
              <div><p style={{ fontWeight: 500 }}>Home layout</p><p className="muted" style={{ fontSize: 13 }}>{resetDone ? "Your home is back to its default layout." : "Put the widgets on your home back where they started."}</p></div>
              <button type="button" className="btn btn-outline" onClick={() => { try { Object.keys(localStorage).filter((k) => k.startsWith("learnly.home.")).forEach((k) => localStorage.removeItem(k)); } catch { /* storage blocked */ } setResetDone(true); }}>Reset home</button>
            </div>
          </section>

          <div>
            <button type="button" className="btn btn-outline" onClick={async () => { await logout.run(); router.replace("/login"); }}>
              <Icon name="logout" size={16} />
              Sign out
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
