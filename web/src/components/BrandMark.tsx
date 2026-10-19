import { clientConfig } from "@/lib/clientConfig";

/** The institution's logo as it is, or, when it has none, the first letter of its name in a tile. */
export function BrandMark({ name, tile, img }: { name: string; tile?: string; img?: string }) {
  return clientConfig.logo ? (
    // eslint-disable-next-line @next/next/no-img-element -- a static file of the build; the desktop export has no image optimizer
    <img src={`/brand/${clientConfig.logo}`} alt="" className={img} />
  ) : (
    <span aria-hidden="true" className={tile}>{name.charAt(0)}</span>
  );
}
