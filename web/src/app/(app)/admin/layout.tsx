"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, type ReactNode } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { useMe } from "@/lib/session";
import s from "@/components/admin/Admin.module.css";

const SECTIONS: [string, string, IconName][] = [
  ["people", "People", "user"],
  ["groups", "Groups", "courses"],
  ["courses", "Terms and courses", "calendar"],
  ["institution", "Institution", "admin"],
  ["audit", "Audit log", "file"],
];

/** The admin area. The links only hide what the server refuses anyway: every /admin call checks the role. */
export default function AdminLayout({ children }: { children: ReactNode }) {
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
        <nav aria-label="Admin" className={s.side}>
          <p className={s.sideTitle}>Admin</p>
          <div className={s.sideNav}>
            {SECTIONS.map(([id, label, icon]) => (
              <Link key={id} href={`/admin/${id}`} className={s.sideLink} aria-current={path === `/admin/${id}` ? "page" : undefined}>
                <Icon name={icon} size={18} />{label}
              </Link>
            ))}
          </div>
        </nav>
        <div className={s.main}>{children}</div>
      </div>
    </div>
  );
}
