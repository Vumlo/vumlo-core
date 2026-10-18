"use client";

import type { ModuleProps } from "../enabled";

/** The UI of the test-only `dummy` module (ADR-0054). It reaches the web bundle only when the module is on. */
export default function DummyModule({ path }: ModuleProps) {
  return <p>vumlo-dummy-module-marker /{path.join("/")}</p>;
}
