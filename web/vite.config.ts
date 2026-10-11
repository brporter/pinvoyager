import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],

  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "src"),
      // The design system is imported from its own directory rather than
      // copied into the app. That is what makes editing a token change the
      // running application instead of changing a snapshot of it — see
      // src/styles/index.css.
      "@design": path.resolve(import.meta.dirname, "..", "design"),
    },
  },

  optimizeDeps: {
    // MapLibre decodes tiles in a Web Worker, which it loads as a sibling file
    // (`maplibre-gl-worker.mjs`) rather than as an import. Vite's dependency
    // pre-bundler rewrites the entry but does not carry the worker across, so
    // the dev server logs
    //
    //   The file does not exist at ".../deps/maplibre-gl-worker.mjs"
    //
    // and the map renders no tiles at all — a gray panel with the pins drawn
    // correctly on top of it, which looks like a tile-server problem rather
    // than a bundler one.
    //
    // Excluding it costs nothing: maplibre-gl ships as a single pre-bundled
    // ESM file, so pre-bundling it again buys no request savings. This affects
    // the dev server only; the production build resolves the worker through
    // Rollup and has always been correct.
    exclude: ["maplibre-gl"],
  },

  server: {
    port: 5173,
    proxy: {
      // The session cookie is HttpOnly and SameSite=Lax, so the browser only
      // sends it to the origin that set it. Proxying the API through the dev
      // server keeps everything on http://localhost:5173, which means sign-in
      // works in development exactly as it will in production. Pointing the
      // app straight at :8080 instead would drop the cookie on every request
      // and look like a broken login.
      "/api": { target: "http://127.0.0.1:8080", changeOrigin: false },
      "/auth": {
        target: "http://127.0.0.1:8080",
        changeOrigin: false,
        // `/auth/callback` is a FRONTEND route — the screen a user lands on
        // after signing in. Every other `/auth/...` path belongs to the API.
        //
        // The two are distinguishable by shape rather than by prefix: the API's
        // callbacks carry a provider segment (`/auth/google/callback`), the
        // frontend's does not. Without this exception, reloading the page after
        // sign-in proxies to the backend and returns 404, and the screen is
        // only ever reachable by clicking through from inside the app.
        bypass: (req) => {
          const path = (req.url ?? "").split("?")[0];
          if (path === "/auth/callback") return "/index.html";
          return undefined;
        },
      },
    },
  },
});
