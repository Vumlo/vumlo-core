"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";
import { useGoogleCallback } from "@/lib/api/hooks/auth";
import { useT } from "@/lib/i18n";
import { once } from "@/lib/once";
import { landingPath } from "@/lib/session";
import s from "../../../auth.module.css";

/** Where Google sends the browser back to. The server checks the code and opens the session (ADR-0017). */
function Callback() {
  const t = useT();
  const router = useRouter();
  const params = useSearchParams();
  const code = params.get("code");
  const state = params.get("state") ?? "";
  const hint = params.get("hint");
  const callback = useGoogleCallback();
  const [failed, setFailed] = useState(!code);

  useEffect(() => {
    if (!code) return;
    let cancelled = false;
    void once(`google:${code}`, () => callback.run(code, state, hint)).then((user) => {
      if (cancelled) return;
      if (user) router.replace(landingPath(user, null));
      else setFailed(true);
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- runs once per code; `callback.run` is stable
  }, [code, state, hint, router]);

  return (
    <div className={s.card}>
      {failed ? (
        <div className={s.sent}>
          <h1 className={s.title}>{t("google.failedTitle")}</h1>
          <p role="alert" className={s.lead} style={{ margin: 0 }}>{callback.error?.detail ?? t("google.failedLead")}</p>
          <Link href="/login" className="btn btn-outline">{t("auth.backToSignIn")}</Link>
        </div>
      ) : (
        <p role="status" className={s.lead} style={{ margin: 0 }}>{t("google.working")}</p>
      )}
    </div>
  );
}

export default function GoogleCallbackPage() {
  return (
    <Suspense fallback={null}>
      <Callback />
    </Suspense>
  );
}
