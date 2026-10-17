const started = new Map<string, Promise<unknown>>();

/**
 * Runs `fn` once per `key` for the life of the page. A link that works only once (an email token, an
 * OAuth code) is spent by the first call, and React strict mode runs effects twice in development.
 */
export function once<T>(key: string, fn: () => Promise<T>): Promise<T> {
  let p = started.get(key) as Promise<T> | undefined;
  if (!p) {
    p = fn();
    started.set(key, p);
  }
  return p;
}
