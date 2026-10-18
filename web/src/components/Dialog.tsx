"use client";

import { useEffect, useId, useRef, type ReactNode } from "react";

type Props = { title: string; onClose: () => void; children: ReactNode; actions?: ReactNode; description?: ReactNode; width?: number };

/** A modal dialog: Escape and a click on the backdrop close it, focus starts inside and returns on close. */
export function Dialog({ title, onClose, children, actions, description, width }: Props) {
  const id = useId();
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const first = ref.current?.querySelector<HTMLElement>("input, select, textarea, [role=radio], button:not([data-close])");
    (first ?? ref.current)?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      if (e.key !== "Tab" || !ref.current) return;
      const focusable = [...ref.current.querySelectorAll<HTMLElement>("a[href], button:not(:disabled), input, select, textarea, [tabindex='0']")];
      if (focusable.length === 0) return;
      const firstEl = focusable[0]!;
      const last = focusable[focusable.length - 1]!;
      if (e.shiftKey && document.activeElement === firstEl) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); firstEl.focus(); }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      previous?.focus();
    };
  }, [onClose]);

  return (
    <div className="scrim" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div ref={ref} role="dialog" aria-modal="true" aria-labelledby={`${id}-h`} tabIndex={-1} className="dialog" style={width ? { maxWidth: width } : undefined}>
        <h2 id={`${id}-h`}>{title}</h2>
        {description && <p className="muted" style={{ fontSize: 15 }}>{description}</p>}
        <div style={{ marginTop: description ? 16 : 12 }}>{children}</div>
        {actions && <div className="dialog-actions">{actions}</div>}
      </div>
    </div>
  );
}
