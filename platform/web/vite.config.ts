import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, type Plugin } from "vite";

// The SPA lives under /app on the platform origin (path-based routing).
// In dev, vite proxies API calls to the local server so the better-auth
// cookie flow works without CORS.
//
// `--mode demo` is the second build path: the same app with the in-browser
// backend (src/demo) swapped in as the entry, no proxy, and its own output
// directory, so it can be published as a static site.
export default defineConfig(({ mode }) => {
  const demo = mode === "demo";
  return {
    base: "/app/",
    plugins: [react(), tailwindcss(), ...(demo ? [demoEntry()] : [])],
    resolve: {
      alias: {
        "@": path.resolve(import.meta.dirname, "./src"),
      },
    },
    build: demo ? { outDir: "dist-demo" } : undefined,
    server: {
      port: demo ? 5175 : 5173,
      // Fail instead of hopping to 5174: only :5173 is a trusted auth origin,
      // so a silently shifted port would break sign-in.
      strictPort: true,
      proxy: demo
        ? undefined
        : {
            "/api": "http://localhost:3000",
          },
    },
  };
});

/// Points index.html at the demo entry, which installs the in-browser
/// backend before loading the real app.
function demoEntry(): Plugin {
  return {
    name: "lightspeed-demo-entry",
    transformIndexHtml: {
      order: "pre",
      handler: (html) => html.replace('src="/src/main.tsx"', 'src="/src/demo/main.ts"'),
    },
  };
}
