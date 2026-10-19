"use client";

import { Suspense } from "react";
import { NotFoundScreen } from "@/components/StateScreen";
import { useModuleAddress } from "@/lib/useRoute";
import { enabledModules } from "@/modules/enabled";

function ModulePage() {
  const { name, path } = useModuleAddress();
  const Module = enabledModules[name];
  return Module ? <Module path={path} /> : <NotFoundScreen />;
}

/** The page of an optional module, at `/m?name=<module>&path=a/b`. Only the modules switched on in vumlo.toml are in the bundle. */
export default function ModuleRoute() {
  return (
    <Suspense fallback={null}>
      <ModulePage />
    </Suspense>
  );
}
