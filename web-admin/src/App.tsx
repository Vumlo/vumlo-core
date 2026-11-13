import { useEffect, type ReactNode } from "react";
import AuthFrame from "@/app/(auth)/layout";
import { appName } from "@/lib/appName";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import { AdminAudit } from "./components/AdminAudit";
import { AdminGroups } from "./components/AdminGroups";
import { AdminInstitution } from "./components/AdminInstitution";
import { AdminPeople } from "./components/AdminPeople";
import { AdminTerms } from "./components/AdminTerms";
import { ConsolePlugins } from "./components/ConsolePlugins";
import { ConsoleShell } from "./components/ConsoleShell";
import { PluginDetail } from "./components/PluginDetail";
import { ChangePassword } from "./pages/ChangePassword";
import { Home } from "./pages/Home";
import { Login } from "./pages/Login";
import { match, useLocation, type Route } from "./router";

/** The pages that need no session: their frame is the plain card of the student app's sign-in. */
const open: Route[] = [
  { path: "/login", render: () => <Login /> },
  { path: "/change-password", render: () => <ChangePassword /> },
];

/** The pages of the console, inside its frame. */
const pages: (Route & { title: MessageKey })[] = [
  { path: "/", title: "console.title", render: () => <Home /> },
  { path: "/people", title: "ap.people", render: () => <AdminPeople /> },
  { path: "/groups", title: "ag.groups", render: () => <AdminGroups /> },
  { path: "/terms", title: "ac.title", render: () => <AdminTerms /> },
  { path: "/plugins", title: "pl.title", render: () => <ConsolePlugins /> },
  { path: "/plugins/:id", title: "pl.title", render: ({ id }) => <PluginDetail id={id ?? ""} /> },
  { path: "/institution", title: "ai.title", render: () => <AdminInstitution /> },
  { path: "/audit", title: "au.title", render: () => <AdminAudit /> },
];

function NotFound() {
  const t = useT();
  return <p className="muted">{t("console.notFound")}</p>;
}

function Titled({ title, children }: { title: MessageKey; children: ReactNode }) {
  const t = useT();
  useEffect(() => {
    document.title = `${t(title)} · ${appName}`;
  }, [t, title]);
  return <>{children}</>;
}

export function App() {
  const { path } = useLocation();
  const clean = path.length > 1 ? path.replace(/\/$/, "") : path;
  for (const route of open) {
    const params = match(route.path, clean);
    if (params) return <Titled title={route.path === "/login" ? "console.login.title" : "change.title"}><AuthFrame>{route.render(params)}</AuthFrame></Titled>;
  }
  for (const route of pages) {
    const params = match(route.path, clean);
    if (params) return <ConsoleShell><Titled title={route.title}>{route.render(params)}</Titled></ConsoleShell>;
  }
  return <ConsoleShell><Titled title="console.title"><NotFound /></Titled></ConsoleShell>;
}
