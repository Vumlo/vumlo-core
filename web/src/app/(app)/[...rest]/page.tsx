import { notFound } from "next/navigation";

/** Any address inside the signed-in frame that no screen owns. It renders (app)/not-found.tsx under the navbar. */
export default function UnknownPage() {
  notFound();
}
