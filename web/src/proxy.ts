import { NextResponse, type NextRequest } from "next/server";
import { clientConfig } from "@/lib/clientConfig";

/**
 * The student app and the admin console are one build, and the address says which one answers
 * (ADR-0095). `VUMLO_ADMIN_HOST` overrides the host of `server.admin_origin`, for a deployment that
 * reaches the process under another name. The desktop export has no proxy and no console
 * (`scripts/build-desktop.mjs`).
 */
const adminHost = (process.env.VUMLO_ADMIN_HOST ?? new URL(clientConfig.public.server.admin_origin).host).toLowerCase();

export function proxy(request: NextRequest) {
  const { pathname } = request.nextUrl;
  const onConsoleHost = request.headers.get("host")?.toLowerCase() === adminHost;
  const inConsole = pathname === "/console" || pathname.startsWith("/console/");
  if (onConsoleHost && !inConsole) {
    // Every student route is the console's here; the API and the files are not routes (see the matcher).
    const url = request.nextUrl.clone();
    url.pathname = "/console";
    url.search = "";
    return NextResponse.redirect(url);
  }
  if (!onConsoleHost && inConsole) {
    // A page that is not there: the not-found page, with its 404.
    const url = request.nextUrl.clone();
    url.pathname = "/not-here";
    url.search = "";
    return NextResponse.rewrite(url);
  }
  return NextResponse.next();
}

// Not the API, Next's own files, the brand files, or anything with a file extension.
export const config = { matcher: ["/((?!api/|_next/|brand/|.*\\..*).*)"] };
