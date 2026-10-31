// A short message that fades by itself, for an outcome no screen shows (a file opened by the desktop app).
// The shell listens (components/shell/Notice.tsx).
type Listener = (text: string) => void;
const listeners = new Set<Listener>();

export function notice(text: string) {
  for (const l of listeners) l(text);
}

export function onNotice(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
