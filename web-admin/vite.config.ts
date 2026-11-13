import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const dir = (path: string) => fileURLToPath(new URL(path, import.meta.url));

// The admin console is a static single-page app (ADR-0095): `vite build` makes `dist/`, which a web server
// serves on the console's own address. It shares the framework-free parts of the student app (the design
// tokens, the API client, the query cache, the catalogues of text, the mock API) by reading `../web/src`,
// and nothing of Next: `next/link` and `next/navigation` are stood in for by `src/shims`.
export default defineConfig(({ mode }) => {
  const mock = mode === "mock";
  return {
    plugins: [react()],
    resolve: {
      alias: [
        { find: /^@console\//, replacement: dir("./src/") },
        { find: /^@\//, replacement: dir("../web/src/") },
        { find: "next/link", replacement: dir("./src/shims/link.tsx") },
        { find: "next/navigation", replacement: dir("./src/shims/navigation.ts") },
        // The desktop app's transport: the console never runs in it.
        { find: "@tauri-apps/api/core", replacement: dir("./src/shims/tauri.ts") },
      ],
      // One copy of these, whichever directory a file that names them sits in.
      dedupe: ["react", "react-dom", "msw"],
    },
    // The shared code asks `process.env.NEXT_PUBLIC_API_MOCKING` whether the mock API is on (it is, under `--mode mock`).
    define: { "process.env.NEXT_PUBLIC_API_MOCKING": JSON.stringify(mock ? "enabled" : "") },
    // The brand files, the logo and the mock service worker of the student app.
    publicDir: dir("../web/public"),
    server: {
      port: 3001,
      strictPort: true,
      fs: { allow: [dir("..")] },
      // The API on the same address, as behind the web server in production (ADR-0013).
      proxy: { "/api": { target: process.env.VUMLO_API_ORIGIN ?? "http://127.0.0.1:8080" } },
    },
    build: { target: "es2022", sourcemap: false },
  };
});
