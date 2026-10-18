"use client";

import { useParams } from "next/navigation";
import { ItemEditor } from "@/components/author/ItemEditor";

export default function EditItemPage() {
  const { courseId, itemId } = useParams<{ courseId: string; itemId: string }>();
  return <ItemEditor courseId={courseId} itemId={itemId} />;
}
