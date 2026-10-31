"use client";

import { useEffect, useState } from "react";
import { Icon } from "@/components/Icon";
import { onNotice } from "@/lib/notice";
import s from "./Shell.module.css";

/** The message `notice()` sent last, for a few seconds. */
export function Notice() {
  const [shown, setShown] = useState<{ id: number; text: string } | null>(null);
  useEffect(() => onNotice((text) => setShown((p) => ({ id: (p?.id ?? 0) + 1, text }))), []);
  useEffect(() => {
    if (!shown) return;
    const timer = setTimeout(() => setShown(null), 4500);
    return () => clearTimeout(timer);
  }, [shown]);
  return (
    <div role="status" aria-live="polite" className={s.notices}>
      {shown && (
        <div className={s.notice}>
          <span aria-hidden="true" className={s.noticeIcon}><Icon name="check" size={16} strokeWidth={2.5} /></span>
          {shown.text}
        </div>
      )}
    </div>
  );
}
