"use client";

import { NotFoundScreen } from "@/components/StateScreen";
import { ItemEditor } from "@/components/author/ItemEditor";
import { useCourseId, useItemId } from "@/lib/useRoute";

export default function EditItemPage() {
  const courseId = useCourseId();
  const itemId = useItemId();
  return itemId ? <ItemEditor courseId={courseId} itemId={itemId} /> : <NotFoundScreen />;
}
