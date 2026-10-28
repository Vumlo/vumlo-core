"use client";

import { useRouter } from "next/navigation";
import { useState, type ReactNode } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { useT } from "@/lib/i18n";
import { useSync } from "@/lib/offline";
import { errorText } from "@/lib/api/client";
import { useLogout } from "@/lib/session";

/** Signing out of the desktop app deletes what was not sent (ADR-0060), so it asks first when something waits. */
export function useSignOut(): { signOut: () => void; dialog: ReactNode } {
  const t = useT();
  const router = useRouter();
  const logout = useLogout();
  const sync = useSync();
  const [asking, setAsking] = useState(false);
  const unsent = (sync?.pending ?? 0) + (sync?.failed ?? 0);

  // Leaving for the sign-in page while the session is still open would only look like signing out.
  async function leave() {
    if (await logout.run()) router.replace("/login");
    else setAsking(true);
  }

  const failed = logout.error !== undefined;
  return {
    signOut: () => (unsent > 0 ? setAsking(true) : void leave()),
    dialog: asking && (
      <ConfirmDialog
        title={failed ? t("account.signOutFailed") : t("offline.signOutTitle")}
        confirm={failed ? t("error.retry") : t("offline.signOutAnyway")}
        danger={!failed}
        pending={logout.pending}
        error={failed ? errorText(logout.error) : undefined}
        onClose={() => { setAsking(false); logout.reset(); }}
        onConfirm={() => void leave()}
      >
        {unsent > 0 ? t.plural("offline.unsent", unsent) : t("account.signOutFailedBody")}
      </ConfirmDialog>
    ),
  };
}
