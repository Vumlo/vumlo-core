import { Icon } from "./Icon";

export type ItemKind = "lecture" | "assignment";

/** The square tile that marks an item as a lecture or an assignment. Used everywhere items show. */
export function ItemTile({ kind, small = false }: { kind: ItemKind; small?: boolean }) {
  return (
    <span aria-hidden="true" className={`tile tile-${kind}${small ? " tile-sm" : ""}`}>
      <Icon name={kind} size={small ? 17 : 19} />
    </span>
  );
}
