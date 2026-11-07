"use client";

import { useEffect, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { useT } from "@/lib/i18n";
import { useBuddyChat } from "./api";
import { Panel } from "./Panel";
import s from "./Buddy.module.css";

/**
 * The button the shell draws on every signed-in page (ADR-0071). It exists only while the server
 * answers the person's chat: for a role the admin did not switch the Buddy on for, that is a 404 and
 * nothing is drawn. The desktop app draws it too: its `api` command carries the calls, and offline
 * (the chat is never cached there) the server does not answer, so nothing is drawn.
 */
export default function Launcher() {
  const t = useT();
  const chat = useBuddyChat();
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        button.current?.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  if (!chat.data) return null;
  return (
    <>
      {open && (
        <div className={s.dock}>
          <Panel
            layout="side"
            onClose={() => {
              setOpen(false);
              button.current?.focus();
            }}
          />
        </div>
      )}
      <button ref={button} type="button" className={s.fab} aria-label={t("buddy.open")} aria-expanded={open} title={t("buddy.open")} onClick={() => setOpen((o) => !o)}>
        <Icon name="chat" size={24} />
      </button>
    </>
  );
}
