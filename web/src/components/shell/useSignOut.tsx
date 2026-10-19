"use client";

import { useRouter } from "next/navigation";
import { useState, type ReactNode } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { useT } from "@/lib/i18n";
import { useSync } from "@/lib/offline";
import { useLogout } from "@/lib/session";

/** Signing out of the desktop app deletes what was not sent (ADR-0060), so it asks first when something waits. */
export function useSignOut(): { signOut: () => void; dialog: ReactNode } {
  const t = useT();
  const router = useRouter();
  const logout = useLogout();
  const sync = useSync();
  const [asking, setAsking] = useState(false);
  const unsent = (sync?.pending ?? 0) + (sync?.failed ?? 0);

  async function leave() {
    await logout.run();
    router.replace("/login");
  }

  return {
    signOut: () => (unsent > 0 ? setAsking(true) : void leave()),
    dialog: asking && (
      <ConfirmDialog title={t("offline.signOutTitle")} confirm={t("offline.signOutAnyway")} danger pending={logout.pending} onClose={() => setAsking(false)} onConfirm={() => void leave()}>
        {t.plural("offline.unsent", unsent)}
      </ConfirmDialog>
    ),
  };
}
