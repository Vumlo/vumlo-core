"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, type ReactNode } from "react";
import { BrandLockup } from "@/components/BrandMark";
import { Icon } from "@/components/Icon";
import { ThemeButton } from "@/components/shell/Shell";
import { appName } from "@/lib/appName";
import { onSessionExpired } from "@/lib/api/client";
import { useT } from "@/lib/i18n";
import { clearQueries, invalidate, retryFailed } from "@/lib/query";
import { fullName, useLogout, useMe } from "@/lib/session";
import a from "@/components/admin/Admin.module.css";
import { SECTIONS } from "./sections";
import s from "./ConsoleShell.module.css";

/** The address with its query string: sign-in must come back to it. */
const here = () => window.location.pathname + window.location.search;

/**
 * The frame of every console page: a bar, a sidebar and a way out. Nobody without a console session
 * sees a page of it; a `401` anywhere sends them to the console's own sign-in, never the students'.
 */
export function ConsoleShell({ children }: { children: ReactNode }) {
  const t = useT();
  const router = useRouter();
  const path = usePathname();
  const me = useMe();
  const logout = useLogout();
  const mustChange = me.data?.must_change_password === true;
  const notAdmin = me.data !== undefined && me.data.role !== "admin";

  useEffect(() => onSessionExpired(() => invalidate("/me")), []);
  useEffect(() => {
    if (me.error?.status === 401 || notAdmin) router.replace(`/console/login?next=${encodeURIComponent(here())}`);
    // A temporary password only opens the page that replaces it (ADR-0017).
    else if (mustChange) router.replace(`/console/change-password?next=${encodeURIComponent(here())}`);
  }, [me.error, notAdmin, mustChange, path, router]);

  async function signOut() {
    if (await logout.run()) {
      clearQueries();
      router.replace("/console/login");
    }
  }

  if (!me.data || mustChange || notAdmin) {
    if (me.error && me.error.status !== 401) {
      return (
        <div className="loading" role="alert">
          <p>{t("shell.accountFailed")}</p>
          <button type="button" className="btn btn-primary" style={{ marginTop: 12 }} onClick={retryFailed}>{t("error.retry")}</button>
        </div>
      );
    }
    return <div className="loading" role="status">{t("common.loading")}</div>;
  }

  return (
    <>
      <header className={s.header}>
        <Link href="/console" className={s.brand} aria-label={appName}>
          <BrandLockup />
        </Link>
        <span className={s.title}>{t("console.title")}</span>
        <span className={s.spacer} />
        <span className={s.who}>{fullName(me.data)}</span>
        <ThemeButton />
        <button type="button" className="btn btn-outline" onClick={() => void signOut()} disabled={logout.pending}>
          <Icon name="logout" size={16} /> {t("account.signOut")}
        </button>
      </header>
      <main className={s.main}>
        <div className="container">
          <div className={a.layout}>
            {SECTIONS.length > 0 && (
              <nav aria-label={t("console.title")} className={a.side}>
                <div className={a.sideNav}>
                  {SECTIONS.map(([id, label, icon]) => (
                    <Link key={id} href={`/console/${id}`} className={a.sideLink} aria-current={path === `/console/${id}` || path.startsWith(`/console/${id}/`) ? "page" : undefined}>
                      <Icon name={icon} size={18} />{t(label)}
                    </Link>
                  ))}
                </div>
              </nav>
            )}
            <div className={a.main}>{children}</div>
          </div>
        </div>
      </main>
    </>
  );
}
