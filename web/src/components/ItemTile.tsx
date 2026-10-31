import { Icon } from "./Icon";

export type ItemKind = "lecture" | "assignment" | "file";

/** The square tile that marks an item as a lecture, an assignment or a file. Used everywhere items show. */
export function ItemTile({ kind, small = false }: { kind: ItemKind; small?: boolean }) {
  return (
    <span aria-hidden="true" className={`tile tile-${kind}${small ? " tile-sm" : ""}`}>
      <Icon name={kind} size={small ? 17 : 19} />
    </span>
  );
}
