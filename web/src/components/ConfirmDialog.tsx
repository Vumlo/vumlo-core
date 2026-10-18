"use client";

import type { ReactNode } from "react";
import { useT } from "@/lib/i18n";
import { Dialog } from "./Dialog";

export function ConfirmDialog({ title, children, confirm, danger, pending, error, onConfirm, onClose }: { title: string; children: ReactNode; confirm: string; danger?: boolean; pending?: boolean; error?: string; onConfirm: () => void; onClose: () => void }) {
  const t = useT();
  return (
    <Dialog
      title={title}
      onClose={onClose}
      actions={
        <>
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
          <button type="button" className={`btn ${danger ? "btn-danger" : "btn-primary"}`} disabled={pending} onClick={onConfirm}>{confirm}</button>
        </>
      }
    >
      <div className="muted" style={{ fontSize: 15 }}>{children}</div>
      {error && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{error}</p>}
    </Dialog>
  );
}
