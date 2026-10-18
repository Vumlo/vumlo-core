"use client";

import Link from "next/link";
import { useMockOutbox } from "@/lib/api/hooks/auth";
import { useT } from "@/lib/i18n";
import s from "@/app/(auth)/auth.module.css";

const mocked = process.env.NEXT_PUBLIC_API_MOCKING === "enabled";

/** Only under `pnpm dev:mock`: stands in for the inbox, so the link in the mail can be followed without a mail server. */
export function MockMailbox({ to }: { to: string }) {
  const t = useT();
  const outbox = useMockOutbox(mocked);
  if (!mocked) return null;
  const mine = (outbox.data?.items ?? []).filter((m) => m.to === to.trim().toLowerCase());
  return (
    <aside className={s.mailbox} aria-label={t("mock.mailbox")}>
      <strong>{t("mock.mailbox")}</strong>
      {mine.length === 0 ? (
        <p style={{ margin: "6px 0 0" }}>{t("mock.noMail")}</p>
      ) : (
        <ul>
          {mine.map((m) => (
            <li key={m.id}><Link href={m.path} className={s.link}>{m.kind === "verify" ? t("mock.verifyLink") : t("mock.resetLink")}</Link></li>
          ))}
        </ul>
      )}
    </aside>
  );
}
