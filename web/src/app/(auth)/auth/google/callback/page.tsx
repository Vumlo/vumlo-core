"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";
import { useGoogleCallback } from "@/lib/api/hooks/auth";
import { clientConfig } from "@/lib/clientConfig";
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
  // A sign-in the desktop app started ends with a link back to the app (ADR-0062).
  const [appLink, setAppLink] = useState<string | null>(null);

  useEffect(() => {
    if (!code) return;
    let cancelled = false;
    void once(`google:${code}`, () => callback.run(code, state, hint)).then((user) => {
      if (cancelled) return;
      if (!user) setFailed(true);
      else if ("handoff" in user) {
        const link = `${clientConfig.desktop.url_scheme}://auth/google?code=${encodeURIComponent(user.handoff)}`;
        setAppLink(link);
        window.location.assign(link);
      } else router.replace(landingPath(user, null));
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- runs once per code; `callback.run` is stable
  }, [code, state, hint, router]);

  if (appLink) {
    return (
      <div className={s.card}>
        <div className={s.sent}>
          <h1 className={s.title}>{t("google.returnTitle")}</h1>
          <p className={s.lead} style={{ margin: 0 }}>{t("google.returnLead")}</p>
          <a href={appLink} className="btn btn-primary">{t("google.returnButton")}</a>
        </div>
      </div>
    );
  }
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
