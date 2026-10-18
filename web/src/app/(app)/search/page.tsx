import { Suspense } from "react";
import { SearchScreen } from "@/components/SearchScreen";

export default function SearchPage() {
  return (
    <Suspense fallback={null}>
      <SearchScreen />
    </Suspense>
  );
}
