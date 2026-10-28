// Stands in for MockMailbox.tsx when the mock API is off (see next.config.ts): a real inbox is the user's own.
export function MockMailbox(_props: { to: string }) {
  return null;
}
