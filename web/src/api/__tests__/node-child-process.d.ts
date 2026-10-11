/**
 * Just enough of `node:child_process` for `contract.test.ts` to spawn `tsc`.
 *
 * Written out rather than pulled in by adding `"node"` to the `types` array in
 * `tsconfig.app.json`: doing that would put Node's globals — `process`,
 * `Buffer`, `__dirname` — in scope for every file in a browser application,
 * where reaching for one is a mistake that would then compile.
 */

declare module "node:child_process" {
  export interface ExecFileSyncOptions {
    cwd?: string;
    encoding?: string;
    stdio?: string;
    timeout?: number;
  }

  /** Throws on a non-zero exit. The thrown value carries `stdout`, which for
   *  `tsc` is where the diagnostics are. */
  export function execFileSync(
    file: string,
    args: readonly string[],
    options?: ExecFileSyncOptions,
  ): string;
}
