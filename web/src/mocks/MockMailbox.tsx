"use client";

import Link from "next/link";
import { api } from "@/lib/api/client";
import type { MockMail } from "@/lib/api/gaps";
import { useLanguage, type Language } from "@/lib/i18n";
import { invalidate, useQuery } from "@/lib/query";
import { useEffect } from "react";
import s from "@/app/(auth)/auth.module.css";

// Its own words, not the app's dictionaries: those ship in every build, and this box does not.
const words: Record<Language, { box: string; none: string; verify: string; reset: string }> = {
  en: { box: "Mock mailbox (dev:mock only)", none: "No mail for this address.", verify: "Confirm the email address", reset: "Choose a new password" },
  uk: { box: "Тестова скринька (лише dev:mock)", none: "Для цієї адреси листів немає.", verify: "Підтвердити адресу пошти", reset: "Вибрати новий пароль" },
};

/**
 * Only under `pnpm dev:mock` (next.config.ts points the import at NoMailbox.tsx otherwise): stands in for the inbox,
 * so the link in the mail can be followed without a mail server. A real server has no `/mock/outbox`.
 */
export function MockMailbox({ to }: { to: string }) {
  const t = words[useLanguage().language];
  // The box appears once the sign-up or reset was sent: ask for the mail that just came.
  useEffect(() => invalidate("/mock/outbox"), []);
  const outbox = useQuery<{ items: MockMail[] }>("/mock/outbox", () => api.get<{ items: MockMail[] }>("/mock/outbox"));
  const mine = (outbox.data?.items ?? []).filter((m) => m.to === to.trim().toLowerCase());
  return (
    <aside className={s.mailbox} aria-label={t.box}>
      <strong>{t.box}</strong>
      {mine.length === 0 ? (
        <p style={{ margin: "8px 0 0" }}>{t.none}</p>
      ) : (
        <ul>
          {mine.map((m) => (
            <li key={m.id}><Link href={m.path} className={s.link}>{m.kind === "verify" ? t.verify : t.reset}</Link></li>
          ))}
        </ul>
      )}
    </aside>
  );
}
