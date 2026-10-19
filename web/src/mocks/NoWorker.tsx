// Stands in for StartWorker.tsx when the mock API is off (see next.config.ts), so msw stays out of the bundle.
export default function NoWorker() {
  return null;
}
