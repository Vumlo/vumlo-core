/** The Vumlo logo: the same in every installation (ADR-0054, amended). */
export function BrandMark({ className }: { className?: string }) {
  // eslint-disable-next-line @next/next/no-img-element -- a static file of the build; the desktop export has no image optimizer
  return <img src="/brand/logo.png" alt="" className={className} />;
}
