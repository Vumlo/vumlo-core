import { orgName } from "@/lib/appName";

/**
 * The Vumlo logo: the mark and the word, the same in every installation (ADR-0054, amended). The word is a mask filled with
 * the ink colour, so it follows the theme. `--h` is the height of the mark (36px; a place with
 * another size sets it); the word is about half of that.
 */
export function BrandLockup() {
  return (
    <span className="lockup">
      {/* eslint-disable-next-line @next/next/no-img-element -- a static file; the desktop export has no image optimizer */}
      <img src="/vumlo-mark.png" alt="" className="lockup-mark" />
      <span aria-hidden="true" className="lockup-word" />
      {orgName && <span className="lockup-org">{orgName}</span>}
    </span>
  );
}
