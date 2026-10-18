"use client";

import { notFound, useParams } from "next/navigation";
import { enabledModules } from "@/modules/enabled";

/** The page of an optional module. Only the modules switched on in vumlo.toml are in the bundle. */
export default function ModulePage() {
  const params = useParams<{ module: string; path?: string[] }>();
  const Module = enabledModules[params.module];
  if (!Module) notFound();
  return <Module path={params.path ?? []} />;
}
