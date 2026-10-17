"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { useT } from "@/lib/i18n";
import s from "./StateScreen.module.css";

/** A whole-page message: a missing page, a failed load, a crash. */
export function StateScreen({ code, title, message, children }: { code?: string; title: string; message: string; children?: ReactNode }) {
  return (
    <section className={s.screen} role="alert">
      {code && <p className={s.code} aria-hidden="true">{code}</p>}
      <h1 className={s.title}>{title}</h1>
      <p className={s.message}>{message}</p>
      {children && <div className={s.actions}>{children}</div>}
    </section>
  );
}

/**
 * A page, course or item that does not exist, or that the person may not open. The two look the same on
 * purpose: a denied view is reported as "not found", so the page does not reveal what exists (AGENTS.md section 5).
 */
export function NotFoundScreen() {
  const t = useT();
  return (
    <StateScreen code="404" title={t("notFound.title")} message={t("notFound.message")}>
      <Link href="/" className="btn btn-primary">{t("notFound.home")}</Link>
      <Link href="/courses" className="btn btn-outline">{t("notFound.courses")}</Link>
    </StateScreen>
  );
}

export function ErrorScreen({ reset }: { reset: () => void }) {
  const t = useT();
  return (
    <StateScreen title={t("error.title")} message={t("error.message")}>
      <button type="button" className="btn btn-primary" onClick={reset}>{t("error.retry")}</button>
      <Link href="/" className="btn btn-outline">{t("notFound.home")}</Link>
    </StateScreen>
  );
}

/** A load that failed for a reason other than "not found": the network, or a server error. */
export function LoadFailed({ title }: { title?: string }) {
  const t = useT();
  return <p className="error-box" role="alert">{title ?? t("error.loadFailed")}</p>;
}
