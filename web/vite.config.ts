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
    // Exclude MapLibre from dependency pre-bundling so its sibling worker module
    // remains resolvable in development. Production bundling handles it separately.
    exclude: ["maplibre-gl"],
  },

  server: {
    port: 5173,
    proxy: {
      // Proxy API requests through the frontend origin so cookie authentication
      // uses the same request origin in development and production.
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
