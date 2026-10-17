/// <reference types="vite/client" />

/**
 * Types for environment variables used by the application. Explicit declarations
 * document the tile configuration values; Vite also permits other environment keys.
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
