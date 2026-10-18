"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, type ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { useMe } from "@/lib/session";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import s from "@/components/admin/Admin.module.css";

const SECTIONS: [string, MessageKey, IconName][] = [
  ["people", "ap.people", "user"],
  ["groups", "ag.groups", "courses"],
  ["courses", "ac.title", "calendar"],
  ["institution", "ai.title", "admin"],
  ["audit", "au.title", "file"],
];

/** The admin area. The links only hide what the server refuses anyway: every /admin call checks the role. */
export default function AdminLayout({ children }: { children: ReactNode }) {
  const t = useT();
  const me = useMe();
  const router = useRouter();
  const path = usePathname();
  const role = me.data?.role;
  useEffect(() => {
    if (role && role !== "admin") router.replace("/");
    if (path === "/admin") router.replace("/admin/people");
  }, [role, path, router]);
  if (role !== "admin") return null;
  return (
    <div className="container">
      <div className={s.layout}>
        <nav aria-label={t("nav.admin")} className={s.side}>
          <p className={s.sideTitle}>{t("nav.admin")}</p>
          <div className={s.sideNav}>
            {SECTIONS.map(([id, label, icon]) => (
              <Link key={id} href={`/admin/${id}`} className={s.sideLink} aria-current={path === `/admin/${id}` ? "page" : undefined}>
                <Icon name={icon} size={18} />{t(label)}
              </Link>
            ))}
          </div>
        </nav>
        <div className={s.main}>{children}</div>
      </div>
    </div>
  );
}
