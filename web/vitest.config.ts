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
      coverage: {
        // Measure every production module, even if no current test imports it.
        include: ["src/**/*.{ts,tsx}"],
        // These files support tests or TypeScript tooling; they are not
        // frontend runtime code and do not belong in the production denominator.
        exclude: [
          "src/**/*.d.ts",
          "src/**/*.{test,spec}.{ts,tsx}",
          "src/**/__tests__/**",
          "src/**/__fixtures__/**",
          "src/**/__mocks__/**",
          "src/test/**",
        ],
        thresholds: {
          statements: 100,
          branches: 100,
          functions: 100,
          lines: 100,
        },
      },
    },
  }),
);
