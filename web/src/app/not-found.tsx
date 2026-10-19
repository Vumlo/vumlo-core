import { NotFoundScreen } from "@/components/StateScreen";
import { Shell } from "@/components/shell/Shell";

/** An address no screen owns: shown inside the signed-in frame, as before (the web server answers it with a 404). */
export default function NotFound() {
  return (
    <Shell>
      <NotFoundScreen />
    </Shell>
  );
}
