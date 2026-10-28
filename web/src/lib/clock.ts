/** The mock server runs on a fixed clock so screens match the design and tests are stable. */
export const MOCK_NOW = new Date("2026-10-17T15:10:00+03:00");

export function now(): Date {
  return process.env.NEXT_PUBLIC_API_MOCKING === "enabled" ? MOCK_NOW : new Date();
}

/**
 * The viewer's own clock, also under the mock. For what is about the person's moment (a greeting), never for
 * deadlines or other data, which stay on `now()` and the institution's time zone.
 */
export function deviceNow(): Date {
  return new Date();
}
