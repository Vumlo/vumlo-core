"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { navFor } from "@/lib/nav";
import { fullName, useLogout, useMe, type User } from "@/lib/session";
import { useTheme } from "@/lib/theme";
import { initials } from "@/lib/format";
import { useInstitution } from "@/lib/api/hooks/institution";
import { useT } from "@/lib/i18n";
import s from "./Shell.module.css";

function AccountMenu({ user }: { user: User }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const router = useRouter();
  const logout = useLogout();
  const t = useT();

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent) {
        if (e.key === "Escape") setOpen(false);
      } else if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", close);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", close);
    };
  }, [open]);

  async function signOut() {
    await logout.run();
    router.replace("/login");
  }

  return (
    <div ref={ref} className={s.actions}>
      <button
        type="button"
        className={s.avatarBtn}
        aria-label={t("account.menu", { name: fullName(user) })}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="avatar">{initials(user.name, user.surname)}</span>
      </button>
      {open && (
        <div role="menu" aria-label={t("account.label")} className={`popover ${s.menu}`}>
          <div className={s.menuHead}>
            <div className={s.menuName}>{fullName(user)}</div>
            <div className={s.menuMeta}>{user.email}</div>
          </div>
          <Link role="menuitem" href="/profile" className="menu-item" onClick={() => setOpen(false)}>
            <Icon name="user" size={16} /> {t("account.profile")}
          </Link>
          <button type="button" role="menuitem" className="menu-item" onClick={signOut}>
            <Icon name="logout" size={16} /> {t("account.signOut")}
          </button>
        </div>
      )}
    </div>
  );
}

export function ThemeButton() {
  const { theme, toggle } = useTheme();
  const t = useT();
  const label = theme === "dark" ? t("theme.toLight") : t("theme.toDark");
  return (
    <button type="button" className="icon-btn" aria-label={label} title={label} onClick={toggle}>
      <Icon name={theme === "dark" ? "sun" : "moon"} size={20} />
    </button>
  );
}

/** The signed-in frame: sticky navbar, phone tabs, and a redirect to /login without a session. */
export function Shell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();
  const me = useMe();
  const t = useT();
  const institution = useInstitution();
  const institutionName = institution.data?.name ?? "Vumlo";
  const mustChange = me.data?.must_change_password === true;

  useEffect(() => {
    if (me.error?.status === 401) router.replace(`/login?next=${encodeURIComponent(pathname)}`);
    // A temporary password only opens the page that replaces it (ADR-0017).
    else if (mustChange) router.replace(`/change-password?next=${encodeURIComponent(pathname)}`);
  }, [me.error, mustChange, pathname, router]);

  if (!me.data || mustChange) {
    return <div className="loading" role="status">{me.error && me.error.status !== 401 ? t("shell.accountFailed") : t("common.loading")}</div>;
  }
  const user = me.data;
  const nav = navFor(user.role);

  function search(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const q = String(new FormData(e.currentTarget).get("q") ?? "").trim();
    router.push(q ? `/search?q=${encodeURIComponent(q)}` : "/search");
  }

  return (
    <>
      <header className={s.header}>
        <Link href="/" className={s.brand}>
          <span aria-hidden="true" className={s.logo}>{institutionName.charAt(0)}</span>
          <span className={s.brandName}>{institutionName}</span>
        </Link>
        <nav aria-label={t("shell.main")} className={s.nav}>
          {nav.map((n) => (
            <Link key={n.href} href={n.href} className={s.navLink} aria-current={n.match(pathname) ? "page" : undefined}>
              {t(n.label)}
            </Link>
          ))}
        </nav>
        <span className={s.spacer} />
        <form role="search" onSubmit={search} className={`search ${s.search}`}>
          <Icon name="search" size={17} />
          <input type="search" name="q" aria-label={t("search.label")} placeholder={t("search.placeholder")} />
        </form>
        <div className={`${s.actions} ${s.desktopOnly}`}>
          <ThemeButton />
        </div>
        <span className={s.phoneOnly}>
          <Link href="/search" className="icon-btn" aria-label={t("search.label")} style={{ width: 44, height: 44 }}>
            <Icon name="search" size={20} />
          </Link>
        </span>
        <AccountMenu user={user} />
      </header>
      <main className={s.main}>{children}</main>
      <nav aria-label={t("shell.main")} className={s.tabs}>
        {nav.map((n) => (
          <Link key={n.href} href={n.href} className={s.tab} aria-current={n.match(pathname) ? "page" : undefined}>
            <span className={s.tabIcon}><Icon name={n.icon} size={21} /></span>
            {t(n.label)}
          </Link>
        ))}
      </nav>
    </>
  );
}
