// A short message that fades by itself, for an outcome no screen shows (a file opened by the desktop app).
// The shell listens (components/shell/Notice.tsx).
export type NoticeTone = "ok" | "error";
type Listener = (text: string, tone: NoticeTone) => void;
const listeners = new Set<Listener>();

export function notice(text: string, tone: NoticeTone = "ok") {
  for (const l of listeners) l(text, tone);
}

export function onNotice(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
