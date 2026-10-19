"use client";

import { NotFoundScreen } from "@/components/StateScreen";
import { ItemView } from "@/components/item/ItemView";
import { useCourseId, useItemId } from "@/lib/useRoute";

export default function ItemPage() {
  const courseId = useCourseId();
  const itemId = useItemId();
  return itemId ? <ItemView courseId={courseId} itemId={itemId} /> : <NotFoundScreen />;
}
