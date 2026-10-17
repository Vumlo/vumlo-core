"use client";

import { useParams } from "next/navigation";
import { ItemView } from "@/components/item/ItemView";

export default function ItemPage() {
  const { courseId, itemId } = useParams<{ courseId: string; itemId: string }>();
  return <ItemView courseId={courseId} itemId={itemId} />;
}
