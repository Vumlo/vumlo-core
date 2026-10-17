"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";
import { useVerifyEmail } from "@/lib/api/hooks/auth";
import { once } from "@/lib/once";
import { useT } from "@/lib/i18n";
import s from "../auth.module.css";

type Result = "working" | "done" | "failed";

/** The page behind the link in the verification mail (ADR-0017). The token is single use. */
function Verify() {
  const t = useT();
  const token = useSearchParams().get("token");
  const verify = useVerifyEmail();
  const [result, setResult] = useState<Result>(token ? "working" : "failed");

  useEffect(() => {
    if (!token) return;
    let cancelled = false;
    void once(`verify:${token}`, () => verify.run(token)).then((ok) => {
      if (!cancelled) setResult(ok ? "done" : "failed");
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- runs once per token; `verify.run` is stable
  }, [token]);

  return (
    <div className={s.card}>
      <div className={s.sent}>
        {result === "working" && <p role="status" className={s.lead} style={{ margin: 0 }}>{t("verify.working")}</p>}
        {result === "done" && (
          <>
            <h1 className={s.title}>{t("verify.doneTitle")}</h1>
            <p className={s.lead} style={{ margin: 0 }}>{t("verify.doneLead")}</p>
            <Link href="/login" className="btn btn-primary btn-lg">{t("login.submit")}</Link>
          </>
        )}
        {result === "failed" && (
          <>
            <h1 className={s.title}>{t("verify.failedTitle")}</h1>
            <p role="alert" className={s.lead} style={{ margin: 0 }}>{t("verify.failedLead")}</p>
            <Link href="/register" className="btn btn-outline">{t("verify.again")}</Link>
          </>
        )}
      </div>
    </div>
  );
}

export default function VerifyEmailPage() {
  return (
    <Suspense fallback={null}>
      <Verify />
    </Suspense>
  );
}
