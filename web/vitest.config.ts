import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config";

// Separate from vite.config.ts because `test` is vitest's key, not vite's —
// putting it in the vite config typechecks against the wrong option type and
// fails the build.
export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: "jsdom",
      globals: true,
      setupFiles: ["./src/test/setup.ts"],
      // Process CSS during tests. The components apply design-system class
      // names, so a test that asserts on a computed style needs the real
      // stylesheet rather than an empty one.
      css: true,
    },
  }),
);
