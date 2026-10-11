/// <reference types="vite/client" />

/**
 * Environment variables this application reads.
 *
 * Declared rather than left to `vite/client`'s index signature, which types
 * every `import.meta.env.X` as `any`. A misspelled variable name would then be
 * `any` too, and would silently be `undefined` at runtime instead of failing to
 * compile.
 */
interface ImportMetaEnv {
  /**
   * Where map tiles come from. Absent in development, in which case
   * `src/map/style.ts` falls back to openstreetmap.org's tile server — see
   * D4a in `docs/architecture/constraints.md`.
   */
  readonly VITE_TILE_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
