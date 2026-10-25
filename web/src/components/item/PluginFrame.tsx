"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { LoadFailed } from "@/components/StateScreen";
import { requestLaunch } from "@/lib/api/hooks/plugins";
import type { Launch, PluginSlot } from "@/lib/api/types";
import { now } from "@/lib/clock";
import { useLanguage, useT } from "@/lib/i18n";
import { readPluginTokens } from "@/lib/pluginTheme";
import { useTheme } from "@/lib/theme";
import s from "./Item.module.css";

const MIN_HEIGHT = 120;
const MAX_HEIGHT = 4000;
/** A token with less than this left is replaced before it is handed over. */
const FRESH_MS = 15_000;

/**
 * The page of a plugin in a sandboxed frame (ADR-0066). This is the only place that touches it.
 *
 * `sandbox="allow-scripts"` and nothing more: the page has an opaque origin, so it has no cookie of
 * ours, no storage and no access to this document, and it cannot navigate the window, open another
 * or submit a form. It is built from the address the core returned and from nothing else.
 *
 * Three messages, all with `v: 1`: the frame says `vumlo.ready` when it listens (and again when it
 * wants a fresh token); this side answers `vumlo.context` with the assignment, a launch token, the
 * theme, the language and the design tokens; the frame may send `vumlo.resize` with its height.
 */
export function PluginFrame({ assignmentId, slot }: { assignmentId: string; slot: PluginSlot }) {
  const t = useT();
  const { theme } = useTheme();
  const { language } = useLanguage();
  const frame = useRef<HTMLIFrameElement>(null);
  const launch = useRef<Launch | null>(null);
  const ready = useRef(false);
  const [height, setHeight] = useState(480);
  const [failed, setFailed] = useState(false);

  const send = useCallback(async () => {
    try {
      if (!launch.current || Date.parse(launch.current.expires_at) - now().getTime() < FRESH_MS) {
        launch.current = await requestLaunch(assignmentId, slot.plugin_id);
      }
    } catch {
      setFailed(true);
      return;
    }
    // The origin of the frame is opaque, so the target has to be "*". Only this window receives it,
    // and it holds ids, a token that lives a minute, and colours: no session, no API token.
    frame.current?.contentWindow?.postMessage(
      { type: "vumlo.context", v: 1, assignment_id: assignmentId, launch: launch.current, theme, language, tokens: readPluginTokens() },
      "*",
    );
  }, [assignmentId, slot.plugin_id, theme, language]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      // Only the frame this component made, whose origin is the opaque "null".
      if (event.source !== frame.current?.contentWindow || event.origin !== "null") return;
      const message: unknown = event.data;
      if (typeof message !== "object" || message === null) return;
      const { type, v, height: asked } = message as { type?: unknown; v?: unknown; height?: unknown };
      if (v !== 1) return;
      if (type === "vumlo.ready") {
        ready.current = true;
        void send();
      } else if (type === "vumlo.resize" && typeof asked === "number" && Number.isInteger(asked)) {
        setHeight(Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, asked)));
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [send]);

  // A change of theme or language: the frame that is listening gets the context again.
  useEffect(() => {
    if (ready.current) void send();
  }, [send]);

  if (failed) return <LoadFailed title={t("plugin.frame.failed")} />;
  return (
    <iframe
      ref={frame}
      className={s.pluginFrame}
      style={{ height }}
      src={slot.url}
      title={t("plugin.frame.title", { name: slot.title })}
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      allow=""
    />
  );
}
