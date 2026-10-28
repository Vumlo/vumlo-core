"use client";

import { useEffect } from "react";

/** Closes a popup menu on Escape or a press outside it. Mark the button and menu wrapper with `data-menu`. */
export function useMenuDismiss(open: boolean, close: () => void) {
  useEffect(() => {
    if (!open) return;
    const onPress = (e: MouseEvent) => {
      if (!(e.target instanceof Element) || !e.target.closest("[data-menu]")) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    document.addEventListener("mousedown", onPress);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPress);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, close]);
}
